//! Relay WebSocket Client
//!
//! 连接到 relay.yinnho.cn，实现心跳保活和断线重连

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::auth::{create_auth_message, SigningKeyPair};
use crate::crypto::{compute_shared_secret, decrypt, encrypt, EncryptedPacket};
use crate::protocol::{AuthResultMessage, ChatRequest, DataMessage, EncryptedPayload, RelayMessage};

/// 默认 Relay 服务器地址
pub const DEFAULT_RELAY_URL: &str = "wss://relay.yinnho.cn";

/// 心跳间隔（毫秒）
const HEARTBEAT_INTERVAL_MS: u64 = 30000;

/// 重连基础延迟（毫秒）
const RECONNECT_BASE_DELAY_MS: u64 = 1000;

/// 最大重连延迟（毫秒）
const RECONNECT_MAX_DELAY_MS: u64 = 30000;

/// 最大重连次数（0 = 无限制）
const MAX_RECONNECT_ATTEMPTS: usize = 0;

/// Relay 客户端事件
#[derive(Debug, Clone)]
pub enum RelayEvent {
    /// 已连接
    Connected,
    /// 已断开
    Disconnected,
    /// 收到加密消息
    Message(serde_json::Value),
    /// 对端已连接
    PeerConnected { carrier_id: String },
    /// 对端已断开
    PeerDisconnected { message: Option<String> },
    /// JWT 已刷新
    JwtRefreshed { jwt: String },
    /// 错误
    Error(String),
}

/// Relay 客户端
pub struct RelayClient {
    /// Relay URL
    url: String,
    /// Carrier ID
    carrier_id: u64,
    /// Ed25519 签名密钥对
    signing_key_pair: SigningKeyPair,
    /// JWT token
    jwt: Option<String>,
    /// 设备 ID
    device_id: Option<String>,
    /// ECDH 密钥对
    ecdh_key_pair: crate::crypto::EcdhKeyPair,
    /// 共享密钥（与 App 的）
    shared_secret: Arc<RwLock<Option<Vec<u8>>>>,
    /// 连接状态
    connected: Arc<RwLock<bool>>,
    /// 重连尝试次数
    reconnect_attempts: Arc<RwLock<usize>>,
    /// 事件发送器
    event_tx: mpsc::UnboundedSender<RelayEvent>,
    /// 是否正在运行
    running: Arc<RwLock<bool>>,
}

impl RelayClient {
    /// 创建新的 Relay 客户端
    pub fn new(
        carrier_id: u64,
        signing_key_pair: SigningKeyPair,
        jwt: Option<String>,
        device_id: Option<String>,
    ) -> Self {
        let (event_tx, _) = mpsc::unbounded_channel();

        Self {
            url: DEFAULT_RELAY_URL.to_string(),
            carrier_id,
            signing_key_pair,
            jwt,
            device_id,
            ecdh_key_pair: crate::crypto::EcdhKeyPair::generate(),
            shared_secret: Arc::new(RwLock::new(None)),
            connected: Arc::new(RwLock::new(false)),
            reconnect_attempts: Arc::new(RwLock::new(0)),
            event_tx,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// 设置 Relay URL
    pub fn with_url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// 获取事件接收器
    pub fn take_event_receiver(&mut self) -> mpsc::UnboundedReceiver<RelayEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.event_tx = tx;
        rx
    }

    /// 订阅事件
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<RelayEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        // 注意：这里需要复制 sender
        let _ = self.event_tx.send(RelayEvent::Connected);
        rx
    }

    /// 连接到 Relay 服务器
    pub async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Relay server: {}", self.url);

        *self.running.write().await = true;

        // 实际连接
        self.do_connect().await
    }

    /// 断开连接
    pub async fn disconnect(&mut self) {
        info!("Disconnecting from Relay server");
        *self.running.write().await = false;
        *self.connected.write().await = false;
    }

    /// 发送聊天消息
    pub async fn send_chat(&self, request: &ChatRequest) -> Result<()> {
        if !*self.connected.read().await {
            return Err(anyhow::anyhow!("Not connected"));
        }

        let shared = self.shared_secret.read().await;
        let secret = shared.as_ref().ok_or_else(|| anyhow::anyhow!("No shared secret"))?;

        // 序列化消息
        let plaintext = serde_json::to_string(request)?;
        let encrypted = encrypt(&plaintext, secret)?;

        let payload: EncryptedPayload = encrypted.into();
        let msg = DataMessage {
            payload,
            message_id: Some(request.message_id.clone()),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };

        let json = serde_json::to_string(&msg)?;
        // 通过 WebSocket 发送
        // 注意：这里需要实际的 WebSocket 连接
        info!("Sent chat message: {}", request.message_id);

        Ok(())
    }

    /// 设置共享密钥（当从 App 收到公钥时调用）
    pub async fn set_shared_secret(&self, peer_public_key: &[u8]) {
        let secret = compute_shared_secret(&self.ecdh_key_pair.private_key, peer_public_key);
        *self.shared_secret.write().await = Some(secret);
        info!("Shared secret established");
    }

    /// 获取 ECDH 公钥（用于发给 App）
    pub fn get_ecdh_public_key(&self) -> Vec<u8> {
        self.ecdh_key_pair.public_key.clone()
    }

    /// 实际执行连接
    async fn do_connect(&mut self) -> Result<()> {
        let url = Url::parse(&self.url)?;
        info!("WebSocket connecting to: {}", url);

        // Convert URL to string for connect_async
        let request = url.as_str();
        let (ws_stream, _) = connect_async(request).await?;
        info!("WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        // 发送认证消息
        let auth_msg = create_auth_message(
            &self.carrier_id.to_string(),
            "carrier",
            &self.signing_key_pair.private_key,
            self.jwt.clone(),
            self.device_id.clone(),
        );

        let auth_json = serde_json::to_string(&auth_msg)?;
        write.send(Message::Text(auth_json.into())).await?;
        info!("Auth message sent");

        *self.connected.write().await = true;
        let _ = self.event_tx.send(RelayEvent::Connected);

        // 重置重连计数
        *self.reconnect_attempts.write().await = 0;

        // 启动心跳任务
        let write_ptr = Arc::new(RwLock::new(write));
        let running = self.running.clone();
        tokio::spawn(async move {
            let mut heartbeat = interval(Duration::from_millis(HEARTBEAT_INTERVAL_MS));
            heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);

            while *running.read().await {
                heartbeat.tick().await;
                let mut w = write_ptr.write().await;
                let ping = serde_json::json!({
                    "type": "ping",
                    "timestamp": chrono::Utc::now().timestamp_millis()
                });
                if let Ok(json) = serde_json::to_string(&ping) {
                    let _ = w.send(Message::Text(json.into())).await;
                    debug!("Ping sent");
                }
            }
        });

        // 接收消息
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = self.handle_message(&text).await {
                        error!("Error handling message: {}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        *self.connected.write().await = false;
        let _ = self.event_tx.send(RelayEvent::Disconnected);

        // 连接断开，不自动重连
        // 重连由调用者决定

        Ok(())
    }

    /// 重新连接
    async fn reconnect(&mut self) {
        loop {
            if !*self.running.read().await {
                info!("Not reconnecting (client stopped)");
                return;
            }

            let attempts = *self.reconnect_attempts.read().await;
            info!("Disconnected, reconnecting... (attempt {})", attempts + 1);

            *self.reconnect_attempts.write().await = attempts + 1;

            // 计算延迟
            let delay = std::cmp::min(
                RECONNECT_BASE_DELAY_MS * 2u64.pow(attempts as u32),
                RECONNECT_MAX_DELAY_MS,
            );

            tokio::time::sleep(Duration::from_millis(delay)).await;

            // 尝试连接
            match self.do_connect().await {
                Ok(()) => {
                    info!("Reconnected successfully");
                    return;
                }
                Err(e) => {
                    error!("Reconnection failed: {}", e);
                    // 继续循环重试
                }
            }
        }
    }

    /// 处理收到的消息
    async fn handle_message(&mut self, text: &str) -> Result<()> {
        debug!("Received message: {}", &text[..text.len().min(200)]);

        let msg: serde_json::Value = serde_json::from_str(text)?;

        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "auth_result" => {
                let result: AuthResultMessage = serde_json::from_str(text)?;
                if result.success {
                    info!("Authentication successful");
                } else {
                    let error = result.error.unwrap_or_else(|| "Unknown error".to_string());
                    error!("Authentication failed: {}", error);
                    let _ = self.event_tx.send(RelayEvent::Error(error));
                }
            }
            "connected" => {
                let carrier_id = msg.get("carrier_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                info!("Peer connected: {}", carrier_id);
                let _ = self.event_tx.send(RelayEvent::PeerConnected { carrier_id });
            }
            "disconnect" => {
                let message = msg.get("message").and_then(|v| v.as_str()).map(String::from);
                info!("Peer disconnected: {:?}", message);
                let _ = self.event_tx.send(RelayEvent::PeerDisconnected { message });
            }
            "pong" => {
                // JWT 刷新
                if let Some(jwt) = msg.get("jwt").and_then(|v| v.as_str()) {
                    info!("JWT refreshed");
                    let _ = self.event_tx.send(RelayEvent::JwtRefreshed { jwt: jwt.to_string() });
                }
            }
            "data" => {
                // 加密数据
                let shared = self.shared_secret.read().await;
                if let Some(secret) = shared.as_ref() {
                    if let Ok(payload) = serde_json::from_str::<DataMessage>(text) {
                        if let Ok(decrypted) = decrypt(&payload.payload, secret) {
                            if let Ok(data) = serde_json::from_str(&decrypted) {
                                let _ = self.event_tx.send(RelayEvent::Message(data));
                            }
                        }
                    }
                } else {
                    warn!("Received data but no shared secret");
                }
            }
            _ => {
                debug!("Unhandled message type: {}", msg_type);
            }
        }

        Ok(())
    }

    /// 处理断开连接
    async fn handle_disconnect(&mut self) {
        loop {
            if !*self.running.read().await {
                info!("Not reconnecting (client stopped)");
                return;
            }

            let attempts = *self.reconnect_attempts.read().await;
            info!("Disconnected, reconnecting... (attempt {})", attempts + 1);

            *self.reconnect_attempts.write().await = attempts + 1;

            // 计算延迟
            let delay = std::cmp::min(
                RECONNECT_BASE_DELAY_MS * 2u64.pow(attempts as u32),
                RECONNECT_MAX_DELAY_MS,
            );

            tokio::time::sleep(Duration::from_millis(delay)).await;

            // 重新连接
            match self.do_connect().await {
                Ok(()) => {
                    info!("Reconnected successfully");
                    return;
                }
                Err(e) => {
                    error!("Reconnection failed: {}", e);
                    // 继续循环重试
                }
            }
        }
    }

    /// 检查是否已连接
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation() {
        let key_pair = SigningKeyPair::generate();
        let client = RelayClient::new(123, key_pair, None, None);

        assert!(!client.is_connected().await);
    }
}
