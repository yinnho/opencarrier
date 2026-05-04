//! WebSocket client for DingTalk Stream (JSON-over-WebSocket protocol).
//!
//! Connects to the DingTalk Stream gateway, performs handshake (CONNECTED → REGISTERED),
//! receives bot message callbacks as JSON text frames, sends ACKs, and echoes keep-alive.

use crate::plugin::channels::dingtalk::api;
use crate::plugin::channels::dingtalk::token::AccessTokenCache;
use crate::plugin::channels::dingtalk::types::*;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use opencarrier_types::plugin::{PluginContent, PluginMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

const DEDUP_TTL: Duration = Duration::from_secs(60);
const DEDUP_MAX_ENTRIES: usize = 10_000;
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub struct DingTalkWsClient {
    tenant_name: String,
    token_cache: Arc<AccessTokenCache>,
    shutdown: Arc<AtomicBool>,
    dedup: DashMap<String, Instant>,
}

impl DingTalkWsClient {
    pub fn new(
        tenant_name: String,
        token_cache: Arc<AccessTokenCache>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            tenant_name,
            token_cache,
            shutdown,
            dedup: DashMap::new(),
        }
    }

    /// Main loop: connect → listen → reconnect on failure.
    pub async fn run(&self, sender: &mpsc::Sender<PluginMessage>) {
        let mut backoff = Duration::from_secs(5);

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                info!(tenant = %self.tenant_name, "Shutdown requested, exiting WS loop");
                return;
            }

            match self.connect_and_listen(sender).await {
                Ok(()) => {
                    if self.shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    warn!(tenant = %self.tenant_name, "WebSocket disconnected, reconnecting...");
                }
                Err(e) => {
                    error!(tenant = %self.tenant_name, "WebSocket error: {e}");
                }
            }

            let check_interval = Duration::from_secs(1);
            let mut waited = Duration::ZERO;
            while waited < backoff {
                if self.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(check_interval).await;
                waited += check_interval;
            }

            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// Connect to WebSocket and listen for events until disconnection.
    async fn connect_and_listen(
        &self,
        sender: &mpsc::Sender<PluginMessage>,
    ) -> Result<(), String> {
        // 1. Get access token
        let token = self
            .token_cache
            .get_token()
            .await
            .map_err(|e| format!("Token error: {e}"))?;

        // 2. Open gateway connection
        let gw = api::open_gateway(
            self.token_cache.http(),
            &token,
            self.token_cache.app_key(),
            self.token_cache.app_secret(),
        )
        .await?;

        let endpoint = gw
            .endpoint
            .ok_or("Missing endpoint in gateway response")?;
        let ticket = gw
            .ticket
            .ok_or("Missing ticket in gateway response")?;

        let ws_url = format!("{endpoint}?ticket={ticket}");

        info!(tenant = %self.tenant_name, "Connecting to DingTalk Stream WebSocket...");

        // 3. Connect WebSocket
        let (ws_stream, _response) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        info!(tenant = %self.tenant_name, "DingTalk WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        // 4. Handshake: wait for CONNECTED then REGISTERED
        let mut registered = false;
        while !registered {
            if self.shutdown.load(Ordering::Relaxed) {
                let _ = write.close().await;
                return Ok(());
            }

            match read.next().await {
                None => return Err("WebSocket closed during handshake".to_string()),
                Some(Ok(Message::Text(text))) => {
                    let frame: WsDownStream = match serde_json::from_str(&text) {
                        Ok(f) => f,
                        Err(e) => {
                            warn!(tenant = %self.tenant_name, "Failed to parse handshake frame: {e}");
                            continue;
                        }
                    };

                    let frame_type = frame.r#type.as_deref().unwrap_or("");
                    let topic = frame
                        .headers
                        .as_ref()
                        .and_then(|h| h.topic.as_deref())
                        .unwrap_or("");

                    info!(
                        tenant = %self.tenant_name,
                        frame_type,
                        topic,
                        "Handshake frame"
                    );

                    if frame_type == "SYSTEM" {
                        match topic {
                            "CONNECTED" => {
                                info!(tenant = %self.tenant_name, "DingTalk Stream CONNECTED");
                            }
                            "REGISTERED" => {
                                info!(tenant = %self.tenant_name, "DingTalk Stream REGISTERED");
                                registered = true;
                            }
                            "ping" => {
                                // Echo back as ACK format: {code:200, headers, message:"OK", data}
                                if let Ok(ack) = serde_json::to_string(&serde_json::json!({
                                    "code": 200,
                                    "headers": frame.headers,
                                    "message": "OK",
                                    "data": frame.data
                                })) {
                                    let _ = write.send(Message::Text(ack)).await;
                                }
                            }
                            "KEEPALIVE" => {
                                // Just reset liveness, no echo needed
                            }
                            "disconnect" => {
                                return Err("Server sent disconnect during handshake".to_string());
                            }
                            _ => {
                                info!(tenant = %self.tenant_name, topic, "Unknown SYSTEM topic during handshake");
                            }
                        }
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    let _ = write.send(Message::Pong(data)).await;
                }
                Some(Ok(Message::Close(_))) => return Err("Close frame during handshake".to_string()),
                Some(Err(e)) => return Err(format!("WebSocket read error during handshake: {e}")),
                _ => {}
            }
        }

        // 5. Main message loop
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                info!(tenant = %self.tenant_name, "Shutdown during WS listen");
                let _ = write.close().await;
                return Ok(());
            }

            match read.next().await {
                None => {
                    warn!(tenant = %self.tenant_name, "WS stream ended");
                    return Ok(());
                }
                Some(Ok(Message::Text(text))) => {
                    self.handle_text_frame(&text, &mut write, sender).await;
                }
                Some(Ok(Message::Ping(data))) => {
                    let _ = write.send(Message::Pong(data)).await;
                }
                Some(Ok(Message::Close(_))) => {
                    warn!(tenant = %self.tenant_name, "WebSocket close frame received");
                    return Ok(());
                }
                Some(Ok(Message::Binary(data))) => {
                    info!(tenant = %self.tenant_name, len = data.len(), "Unexpected binary frame");
                }
                Some(Err(e)) => {
                    return Err(format!("WebSocket read error: {e}"));
                }
                _ => {}
            }
        }
    }

    /// Handle a JSON text frame from the WebSocket.
    async fn handle_text_frame(
        &self,
        text: &str,
        write: &mut futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        sender: &mpsc::Sender<PluginMessage>,
    ) {
        let frame: WsDownStream = match serde_json::from_str(text) {
            Ok(f) => f,
            Err(e) => {
                warn!(tenant = %self.tenant_name, "Failed to parse WS frame: {e}");
                return;
            }
        };

        let frame_type = frame.r#type.as_deref().unwrap_or("");
        let message_id = frame
            .headers
            .as_ref()
            .and_then(|h| h.message_id.clone())
            .or(frame.message_id.clone())
            .unwrap_or_default();

        let topic = frame
            .headers
            .as_ref()
            .and_then(|h| h.topic.as_deref())
            .unwrap_or("");

        match frame_type {
            "SYSTEM" => {
                match topic {
                    "ping" => {
                        // Echo back as ACK format: {code:200, headers, message:"OK", data}
                        if let Ok(ack) = serde_json::to_string(&serde_json::json!({
                            "code": 200,
                            "headers": frame.headers,
                            "message": "OK",
                            "data": frame.data
                        })) {
                            if let Err(e) = write.send(Message::Text(ack)).await {
                                warn!(tenant = %self.tenant_name, "Ping echo failed: {e}");
                            }
                        }
                    }
                    "KEEPALIVE" => {
                        // Just reset liveness, no response needed
                    }
                    "disconnect" => {
                        warn!(tenant = %self.tenant_name, "Server sent disconnect");
                    }
                    _ => {
                        info!(
                            tenant = %self.tenant_name,
                            topic,
                            "SYSTEM frame"
                        );
                    }
                }
            }
            "CALLBACK" => {
                let topic = frame
                    .headers
                    .as_ref()
                    .and_then(|h| h.topic.as_deref())
                    .unwrap_or("");

                if topic == TOPIC_ROBOT {
                    self.dispatch_callback(&message_id, &frame, write, sender)
                        .await;
                } else {
                    info!(tenant = %self.tenant_name, topic, "Ignoring unknown callback topic");
                    // ACK even unknown topics
                    if !message_id.is_empty() {
                        let ack = WsAck::for_message(&message_id);
                        if let Ok(ack_json) = serde_json::to_string(&ack) {
                            let _ = write.send(Message::Text(ack_json)).await;
                        }
                    }
                }
            }
            _ => {
                info!(tenant = %self.tenant_name, frame_type, "Unknown frame type");
            }
        }
    }

    /// Process a bot message callback.
    async fn dispatch_callback(
        &self,
        message_id: &str,
        frame: &WsDownStream,
        write: &mut futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        sender: &mpsc::Sender<PluginMessage>,
    ) {
        // Send ACK immediately
        if !message_id.is_empty() {
            let ack = WsAck::for_message(message_id);
            if let Ok(ack_json) = serde_json::to_string(&ack) {
                if let Err(e) = write.send(Message::Text(ack_json)).await {
                    warn!(tenant = %self.tenant_name, "ACK send failed: {e}");
                }
            }
        }

        // Parse callback data
        let data_str = match frame.data.as_deref() {
            Some(d) if !d.is_empty() => d,
            _ => return,
        };

        let msg: DingTalkInboundMessage = match serde_json::from_str(data_str) {
            Ok(m) => m,
            Err(e) => {
                warn!(tenant = %self.tenant_name, "Failed to parse callback data: {e}");
                return;
            }
        };

        // Only handle text messages
        if msg.msgtype.as_deref() != Some("text") {
            return;
        }

        let content = msg
            .text
            .as_ref()
            .and_then(|t| t.content.as_deref())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            return;
        }

        // Dedup by message_id
        if !message_id.is_empty() {
            if self.dedup.contains_key(message_id) {
                return;
            }
            self.evict_old_entries();
            self.dedup.insert(message_id.to_string(), Instant::now());
        }

        let sender_id = msg.sender_id.clone().unwrap_or_default();
        let sender_nick = msg.sender_nick.clone().unwrap_or_else(|| sender_id.clone());
        let is_group = msg.conversation_type.as_deref() == Some("2");
        let conversation_id = msg.conversation_id.clone();

        info!(
            tenant = %self.tenant_name,
            from = %sender_id,
            nick = %sender_nick,
            is_group,
            text_len = content.len(),
            "Inbound DingTalk message"
        );

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let plugin_msg = PluginMessage {
            channel_type: "dingtalk".to_string(),
            platform_message_id: message_id.to_string(),
            sender_id: sender_id.clone(),
            sender_name: sender_nick,
            tenant_id: self.tenant_name.clone(),
            content: PluginContent::Text(content),
            timestamp_ms: now_ms,
            is_group,
            thread_id: conversation_id,
            metadata: Default::default(),
        };

        let _ = sender.try_send(plugin_msg);
    }

    fn evict_old_entries(&self) {
        let now = Instant::now();
        self.dedup
            .retain(|_, received_at| now.duration_since(*received_at) < DEDUP_TTL);

        if self.dedup.len() > DEDUP_MAX_ENTRIES {
            let to_remove: Vec<String> = self
                .dedup
                .iter()
                .take(self.dedup.len() - DEDUP_MAX_ENTRIES)
                .map(|r| r.key().clone())
                .collect();
            for key in to_remove {
                self.dedup.remove(&key);
            }
        }
    }
}
