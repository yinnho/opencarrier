//! WebSocket event listener for Feishu/Lark (v2 binary protocol).
//!
//! Connects to the Feishu WebSocket endpoint, receives real-time events
//! encoded as protobuf `pbbp2.Frame` binary messages, parses
//! `im.message.receive_v1` events, and dispatches them to the host.
//!
//! Protocol: All frames are binary protobuf (not JSON text).
//! - Application-level ping/pong (method=0, header type=ping/pong)
//! - Event data frames (method=1) must be ACKed
//! - Large events may be fragmented (message_id + seq + sum headers)

use crate::plugin::channels::feishu::api;
use crate::plugin::channels::feishu::pbbp2::*;
use crate::plugin::channels::feishu::token::TenantTokenCache;
use crate::plugin::channels::feishu::types::*;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use opencarrier_types::plugin::{PluginContent, PluginMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

/// Message deduplication map: message_id → received_at.
const DEDUP_TTL: Duration = Duration::from_secs(300);

/// Maximum reconnection backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Default ping interval (server may override via pong payload).
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(120);

pub struct FeishuWsClient {
    tenant_name: String,
    bot_uuid: String,
    token_cache: Arc<TenantTokenCache>,
    shutdown: Arc<AtomicBool>,
    dedup: DashMap<String, Instant>,
}

impl FeishuWsClient {
    pub fn new(
        tenant_name: String,
        bot_uuid: String,
        token_cache: Arc<TenantTokenCache>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            tenant_name,
            bot_uuid,
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

            // Wait with shutdown check
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

    /// Connect to the WebSocket and listen for events until disconnection.
    async fn connect_and_listen(&self, sender: &mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let ws_resp = api::get_ws_endpoint(
            self.token_cache.http(),
            self.token_cache.app_id(),
            self.token_cache.app_secret(),
            self.token_cache.api_base(),
        )
        .await?;

        if ws_resp.code != 0 {
            return Err(format!("ws/endpoint error: code={} msg={}", ws_resp.code, ws_resp.msg));
        }

        let ws_url = ws_resp
            .data
            .and_then(|d| d.url)
            .ok_or("Missing URL in ws/endpoint response")?;

        // Extract service_id from URL query params (needed for ping frames).
        let service_id = parse_service_id(&ws_url).unwrap_or(0);

        info!(
            tenant = %self.tenant_name,
            service_id,
            "Connecting to Feishu WebSocket..."
        );

        let (ws_stream, _response) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        info!(tenant = %self.tenant_name, "Feishu WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        let mut ping_interval = tokio::time::interval(DEFAULT_PING_INTERVAL);
        ping_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut fragments = FragmentCache::new();

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                info!(tenant = %self.tenant_name, "Shutdown during WS listen");
                let _ = write.close().await;
                return Ok(());
            }

            tokio::select! {
                msg_result = read.next() => {
                    match msg_result {
                        None => {
                            warn!(tenant = %self.tenant_name, "WS stream ended");
                            return Ok(());
                        }
                        Some(Ok(Message::Binary(data))) => {
                            let hex_preview: Vec<String> = data.iter().take(50).map(|b| format!("{b:02x}")).collect();
                            info!(tenant = %self.tenant_name, len = data.len(), hex = %hex_preview.join(""), "WS binary frame received");
                            self.handle_binary_frame(&data, &mut write, &mut fragments, sender).await;
                        }
                        Some(Ok(Message::Text(text))) => {
                            info!(tenant = %self.tenant_name, len = text.len(), "WS text frame received");
                        }
                        Some(Ok(Message::Ping(data))) => {
                            info!(tenant = %self.tenant_name, len = data.len(), "WS protocol ping received");
                        }
                        Some(Ok(Message::Pong(data))) => {
                            info!(tenant = %self.tenant_name, len = data.len(), "WS protocol pong received");
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!(tenant = %self.tenant_name, "WebSocket close frame received");
                            return Ok(());
                        }
                        Some(Ok(Message::Frame(_))) => {
                            info!(tenant = %self.tenant_name, "WS raw frame received");
                        }
                        Some(Err(e)) => {
                            return Err(format!("WebSocket read error: {e}"));
                        }
                    }
                }
                _ = ping_interval.tick() => {
                    let ping = Pbbp2Frame::ping(service_id);
                    let encoded = ping.encode();
                    let hex: Vec<String> = encoded.iter().map(|b| format!("{b:02x}")).collect();
                    info!(tenant = %self.tenant_name, len = encoded.len(), hex = %hex.join(""), "Sending app-level ping");
                    if let Err(e) = write.send(Message::Binary(encoded.into())).await {
                        return Err(format!("WS ping send failed: {e}"));
                    }
                }
            }
        }
    }

    /// Handle a binary frame (protobuf pbbp2.Frame).
    async fn handle_binary_frame(
        &self,
        data: &[u8],
        write: &mut futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        fragments: &mut FragmentCache,
        sender: &mpsc::Sender<PluginMessage>,
    ) {
        let frame = match Pbbp2Frame::decode(data) {
            Some(f) => f,
            None => {
                warn!(tenant = %self.tenant_name, len = data.len(), "Failed to decode binary frame as pbbp2.Frame");
                return;
            }
        };

        match frame.method {
            METHOD_CONTROL => self.handle_control(&frame),
            METHOD_DATA => {
                self.handle_data(&frame, write, fragments, sender).await;
            }
            _ => {
                warn!(tenant = %self.tenant_name, method = frame.method, "Unknown frame method");
            }
        }
    }

    /// Handle control frame (pong, handshake).
    fn handle_control(&self, frame: &Pbbp2Frame) {
        let frame_type = frame.header("type").unwrap_or("");

        match frame_type {
            "pong" => {
                let payload_str = String::from_utf8_lossy(&frame.payload);
                info!(
                    tenant = %self.tenant_name,
                    payload = %payload_str,
                    "WS pong received"
                );
            }
            "" => {
                // Handshake response — check status
                if let Some(status) = frame.header("handshake-status") {
                    let msg = frame.header("handshake-msg").unwrap_or("");
                    info!(
                        tenant = %self.tenant_name,
                        status,
                        msg,
                        "WS handshake"
                    );
                }
            }
            other => {
                info!(tenant = %self.tenant_name, frame_type = other, "Unknown control frame type");
            }
        }
    }

    /// Handle data frame (event delivery).
    async fn handle_data(
        &self,
        frame: &Pbbp2Frame,
        write: &mut futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        fragments: &mut FragmentCache,
        sender: &mpsc::Sender<PluginMessage>,
    ) {
        let start = std::time::Instant::now();

        let message_id = frame.header("message_id").unwrap_or("").to_string();
        let seq: usize = frame
            .header("seq")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let sum: usize = frame
            .header("sum")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let payload = match fragments.add(&message_id, seq, sum, frame.payload.clone()) {
            Some(p) => p,
            None => {
                // Still collecting fragments — send ACK for this part
                let ack = Pbbp2Frame::ack_for(frame, start.elapsed().as_millis() as i64);
                if let Err(e) = write.send(Message::Binary(ack.encode().into())).await {
                    warn!(tenant = %self.tenant_name, "ACK send failed: {e}");
                }
                return;
            }
        };

        // Send ACK for complete event
        let ack = Pbbp2Frame::ack_for(frame, start.elapsed().as_millis() as i64);
        if let Err(e) = write.send(Message::Binary(ack.encode().into())).await {
            warn!(tenant = %self.tenant_name, "ACK send failed: {e}");
        }

        let payload_str = String::from_utf8_lossy(&payload);
        info!(
            tenant = %self.tenant_name,
            payload_len = payload.len(),
            payload_preview = %&payload_str[..payload_str.len().min(300)],
            "WS event data received"
        );

        self.dispatch_event(&payload_str, sender);
    }

    /// Parse event payload and dispatch to handler.
    fn dispatch_event(&self, payload_str: &str, sender: &mpsc::Sender<PluginMessage>) {
        let event: serde_json::Value = match serde_json::from_str(payload_str) {
            Ok(v) => v,
            Err(e) => {
                warn!(tenant = %self.tenant_name, "Failed to parse event JSON: {e}");
                return;
            }
        };

        // The SDK wraps the event in {"event": {...}, "header": {...}}
        // but the payload from pbbp2.Frame IS the event body directly.
        // Check for im.message.receive_v1 structure.
        let header = event.get("header");
        let event_type = header
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if event_type != "im.message.receive_v1" {
            info!(tenant = %self.tenant_name, event_type, "Ignoring non-message event");
            return;
        }

        let message = match event.get("event").and_then(|e| e.get("message")) {
            Some(m) => m,
            None => return,
        };

        let msg_id = match message.get("message_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => return,
        };

        // Dedup
        if self.dedup.contains_key(msg_id) {
            return;
        }
        self.evict_old_entries();
        self.dedup.insert(msg_id.to_string(), Instant::now());

        if message.get("message_type").or_else(|| message.get("msg_type")).and_then(|v| v.as_str()) != Some("text") {
            return;
        }

        let text = message
            .get("content")
            .and_then(|c| c.as_str())
            .and_then(|c| serde_json::from_str::<TextContent>(c).ok())
            .and_then(|tc| tc.text)
            .unwrap_or_default();

        let sender_obj = event.get("event").and_then(|e| e.get("sender"));
        let sender_id_obj = sender_obj.and_then(|s| s.get("sender_id"));
        let (sender_id, sender_name) = match sender_id_obj {
            Some(sid) => {
                let open_id = sid.get("open_id").and_then(|v| v.as_str()).unwrap_or("");
                (open_id.to_string(), open_id.to_string())
            }
            None => return,
        };

        let chat_type = message
            .get("chat_type")
            .and_then(|v| v.as_str())
            .unwrap_or("p2p");
        let is_group = chat_type == "group";

        let create_time_ms = message
            .get("create_time")
            .and_then(|v| v.as_str())
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            });

        info!(
            tenant = %self.tenant_name,
            from = %sender_id,
            chat_type,
            text_len = text.len(),
            "Inbound Feishu message"
        );

        let plugin_msg = PluginMessage {
            channel_type: "feishu".to_string(),
            platform_message_id: msg_id.to_string(),
            sender_id: sender_id.clone(),
            sender_name,
            tenant_id: self.bot_uuid.clone(),
            content: PluginContent::Text(text),
            timestamp_ms: create_time_ms,
            is_group,
            thread_id: message.get("chat_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
            metadata: Default::default(),
        };

        let _ = sender.try_send(plugin_msg);
    }

    /// Remove dedup entries older than TTL.
    fn evict_old_entries(&self) {
        let now = Instant::now();
        self.dedup
            .retain(|_, received_at| now.duration_since(*received_at) < DEDUP_TTL);
    }
}

/// Parse `service_id` from the WS URL query parameters.
fn parse_service_id(url: &str) -> Option<i32> {
    url.split("service_id=")
        .nth(1)?
        .split('&')
        .next()
        .and_then(|v| v.parse().ok())
}
