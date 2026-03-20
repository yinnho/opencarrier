//! Relay Message Protocol
//!
//! 定义与 relay.yinnho.cn 通信的消息格式

use serde::{Deserialize, Serialize};

/// 基础消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RelayMessage {
    /// 认证消息
    Auth(AuthMessage),
    /// 认证结果
    AuthResult(AuthResultMessage),
    /// 加密数据
    Data(DataMessage),
    /// Ping（心跳）
    Ping { timestamp: i64 },
    /// Pong（心跳响应）
    Pong { timestamp: i64, jwt: Option<String> },
    /// 对端已连接
    Connected { carrier_id: String },
    /// 对端已断开
    Disconnect { message: Option<String> },
    /// 中继消息
    Relay(RelayDataMessage),
    /// 错误
    Error { error: String },
}

/// 认证消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMessage {
    pub carrier_id: String,
    pub role: String,
    pub timestamp: i64,
    pub signature: String,
    pub jwt: Option<String>,
    pub device_id: Option<String>,
}

/// 认证结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResultMessage {
    pub success: bool,
    pub message: Option<String>,
    pub error: Option<String>,
}

/// 加密数据消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataMessage {
    pub payload: EncryptedPayload,
    pub message_id: Option<String>,
    #[serde(default)]
    pub timestamp: i64,
}

/// 加密负载
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPayload {
    pub version: u8,
    pub timestamp: i64,
    pub nonce: String,
    pub ciphertext: String,
    pub tag: String,
}

/// 中继数据消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayDataMessage {
    pub to: String,
    pub from: Option<String>,
    pub payload: EncryptedPayload,
    pub message_id: Option<String>,
}

/// 心跳消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingMessage {
    pub timestamp: i64,
}

/// Chat 消息请求（用于发送聊天）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub conversation_type: String,
    pub chat_type: String,
    pub content: String,
    pub avatar_id: Option<String>,
    pub plugin_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub message_id: String,
}

/// Chat 消息响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    #[serde(rename = "conversationId")]
    pub conversation_id: String,
    #[serde(rename = "conversationType")]
    pub conversation_type: String,
    #[serde(rename = "chatType")]
    pub chat_type: String,
    pub content: String,
    #[serde(rename = "messageId")]
    pub message_id: Option<String>,
}

/// 流式消息块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    #[serde(rename = "conversationId")]
    pub conversation_id: String,
    pub delta: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    pub done: bool,
}

/// 错误消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessage {
    pub error: String,
}
