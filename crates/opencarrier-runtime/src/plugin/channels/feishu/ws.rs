//! WebSocket event listener for Feishu/Lark.
//!
//! Connects to the Feishu WebSocket endpoint, receives real-time events,
//! parses `im.message.receive_v1` messages, and dispatches them to the host.

use crate::plugin::channels::feishu::api;
use crate::plugin::channels::feishu::token::TenantTokenCache;
use crate::plugin::channels::feishu::types::*;
use dashmap::DashMap;
use futures::StreamExt;
use opencarrier_types::plugin::{PluginContent, PluginMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

/// Message deduplication map: message_id → received_at.
/// Entries older than 5 minutes are evicted on read.
const DEDUP_TTL: Duration = Duration::from_secs(300);

/// Maximum reconnection backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub struct FeishuWsClient {
    tenant_name: String,
    token_cache: Arc<TenantTokenCache>,
    shutdown: Arc<AtomicBool>,
    dedup: DashMap<String, Instant>,
}

impl FeishuWsClient {
    pub fn new(
        tenant_name: String,
        token_cache: Arc<TenantTokenCache>,
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
                    // Clean disconnect (shutdown or server close)
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

            // Exponential backoff
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// Connect to the WebSocket and listen for events until disconnection.
    async fn connect_and_listen(&self, sender: &mpsc::Sender<PluginMessage>) -> Result<(), String> {
        // Get token
        let token = self.token_cache.get_token().await?;

        // Get WebSocket endpoint URL
        let ws_resp = api::get_ws_endpoint(self.token_cache.http(), &token, self.token_cache.api_base()).await?;
        if ws_resp.code != 0 {
            return Err(format!("ws/endpoint error: code={} msg={}", ws_resp.code, ws_resp.msg));
        }
        let ws_url = ws_resp
            .data
            .and_then(|d| d.endpoint)
            .ok_or("Missing endpoint in ws/endpoint response")?;

        info!(tenant = %self.tenant_name, "Connecting to Feishu WebSocket...");

        // Connect WebSocket
        let (ws_stream, _response) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        info!(tenant = %self.tenant_name, "Feishu WebSocket connected");

        let (_write, mut read) = ws_stream.split();

        // Reset backoff on successful connection
        while let Some(msg_result) = read.next().await {
            if self.shutdown.load(Ordering::Relaxed) {
                info!(tenant = %self.tenant_name, "Shutdown during WS listen");
                return Ok(());
            }

            match msg_result {
                Ok(Message::Text(text)) => {
                    self.handle_frame(&text, sender);
                }
                Ok(Message::Ping(data)) => {
                    // tungstenite auto-replies pings
                    let _ = data;
                }
                Ok(Message::Close(_)) => {
                    warn!(tenant = %self.tenant_name, "WebSocket close frame received");
                    return Ok(());
                }
                Err(e) => {
                    return Err(format!("WebSocket read error: {e}"));
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Parse a WebSocket text frame and dispatch messages.
    fn handle_frame(&self, text: &str, sender: &mpsc::Sender<PluginMessage>) {
        let frame: WsEventFrame = match serde_json::from_str(text) {
            Ok(f) => f,
            Err(_) => return, // Non-event frames (e.g. heartbeat ack)
        };

        let header = match frame.header {
            Some(ref h) => h,
            None => return,
        };

        let event_type = match header.event_type.as_deref() {
            Some(t) => t,
            None => return,
        };

        // Only handle message receive events in Phase 1
        if event_type != "im.message.receive_v1" {
            return;
        }

        let payload_str = match frame.payload.as_deref() {
            Some(p) => p,
            None => return,
        };

        let event: MessageReceiveEvent = match serde_json::from_str(payload_str) {
            Ok(e) => e,
            Err(e) => {
                warn!(tenant = %self.tenant_name, "Failed to parse message event: {e}");
                return;
            }
        };

        let message = match event.message {
            Some(ref m) => m,
            None => return,
        };

        let msg_id = match message.message_id.as_deref() {
            Some(id) if !id.is_empty() => id,
            _ => return,
        };

        // Dedup
        if self.dedup.contains_key(msg_id) {
            return;
        }
        self.evict_old_entries();
        self.dedup.insert(msg_id.to_string(), Instant::now());

        // Only handle text messages in Phase 1
        if message.msg_type.as_deref() != Some("text") {
            return;
        }

        // Extract text content
        let text = message
            .content
            .as_deref()
            .and_then(|c| serde_json::from_str::<TextContent>(c).ok())
            .and_then(|tc| tc.text)
            .unwrap_or_default();

        // Get sender info
        let (sender_id, sender_name) = match event.sender.as_ref().and_then(|s| s.sender_id.as_ref()) {
            Some(sid) => (
                sid.open_id.clone().unwrap_or_default(),
                sid.open_id.clone().unwrap_or_default(),
            ),
            None => return,
        };

        let chat_type = message.chat_type.as_deref().unwrap_or("p2p");
        let is_group = chat_type == "group";

        let create_time_ms = message
            .create_time
            .as_deref()
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
            tenant_id: self.tenant_name.clone(),
            content: PluginContent::Text(text),
            timestamp_ms: create_time_ms,
            is_group,
            thread_id: message.chat_id.clone(),
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
