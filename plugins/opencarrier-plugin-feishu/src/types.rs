//! Feishu/Lark API type definitions.
//!
//! Covers: tenant_access_token, WebSocket event frames, message send/reply.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Feishu (China) API base URL.
pub const FEISHU_API_BASE: &str = "https://open.feishu.cn";
/// Lark (International) API base URL.
pub const LARK_API_BASE: &str = "https://open.larksuite.com";

/// Token refresh safety margin (refresh 5 minutes before expiry).
pub const TOKEN_REFRESH_AHEAD_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Tenant Access Token
// ---------------------------------------------------------------------------

/// Request body for `POST /open-apis/auth/v3/tenant_access_token/internal`.
#[derive(Debug, Clone, Serialize)]
pub struct TenantTokenRequest {
    pub app_id: String,
    pub app_secret: String,
}

/// Response from tenant_access_token endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct TenantTokenResponse {
    pub code: i64,
    pub msg: String,
    pub tenant_access_token: Option<String>,
    pub expire: Option<u64>,
}

// ---------------------------------------------------------------------------
// Send Message
// ---------------------------------------------------------------------------

/// Request body for `POST /open-apis/im/v1/messages`.
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub receive_id: String,
    pub msg_type: String,
    pub content: String,
}

/// Response from send message endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SendMessageResponse {
    pub code: i64,
    pub msg: String,
    pub data: Option<SendMessageData>,
}

/// Data payload in a send/reply message response.
#[derive(Debug, Clone, Deserialize)]
pub struct SendMessageData {
    pub message_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Reply Message
// ---------------------------------------------------------------------------

/// Request body for `POST /open-apis/im/v1/messages/{message_id}/reply`.
#[derive(Debug, Clone, Serialize)]
pub struct ReplyMessageRequest {
    pub content: String,
    pub msg_type: String,
}

// ---------------------------------------------------------------------------
// WebSocket endpoint
// ---------------------------------------------------------------------------

/// Response from `POST /open-apis/callback/ws/endpoint`.
#[derive(Debug, Clone, Deserialize)]
pub struct WsEndpointResponse {
    pub code: i64,
    pub msg: String,
    pub data: Option<WsEndpointData>,
}

/// WebSocket connection URL payload.
#[derive(Debug, Clone, Deserialize)]
pub struct WsEndpointData {
    pub endpoint: Option<String>,
    pub expire_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// WebSocket event frames
// ---------------------------------------------------------------------------

/// Top-level frame received from the Feishu WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct WsEventFrame {
    pub header: Option<WsEventHeader>,
    /// JSON-encoded event payload string.
    pub payload: Option<String>,
}

/// Event header (envelope metadata).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WsEventHeader {
    pub app_id: Option<String>,
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub create_time: Option<String>,
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// im.message.receive_v1 event payload
// ---------------------------------------------------------------------------

/// Root of a `im.message.receive_v1` event.
#[derive(Debug, Clone, Deserialize)]
pub struct MessageReceiveEvent {
    pub message: Option<MessageContent>,
    pub sender: Option<Sender>,
}

/// Message content within a receive event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageContent {
    pub message_id: Option<String>,
    pub root_id: Option<String>,
    pub chat_id: Option<String>,
    pub chat_type: Option<String>,
    pub msg_type: Option<String>,
    pub content: Option<String>,
    pub create_time: Option<String>,
    pub update_time: Option<String>,
}

/// Sender information within a receive event.
#[derive(Debug, Clone, Deserialize)]
pub struct Sender {
    pub sender_id: Option<SenderId>,
    pub sender_type: Option<String>,
    pub tenant_key: Option<String>,
}

/// Sender identifiers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SenderId {
    pub open_id: Option<String>,
    pub user_id: Option<String>,
    pub union_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Text message content (parsed from JSON string in MessageContent.content)
// ---------------------------------------------------------------------------

/// Parsed text message body: `{"text": "hello"}`.
#[derive(Debug, Clone, Deserialize)]
pub struct TextContent {
    pub text: Option<String>,
}

// ---------------------------------------------------------------------------
// Tenant configuration (read from plugin.toml [[tenants]])
// ---------------------------------------------------------------------------

/// Per-tenant configuration parsed from plugin.toml.
#[derive(Debug, Clone)]
pub struct FeishuTenantConfig {
    pub name: String,
    pub app_id: String,
    pub app_secret: String,
    /// "feishu" (China) or "lark" (International).
    pub brand: String,
}

impl FeishuTenantConfig {
    /// Get the API base URL for this tenant's brand.
    pub fn api_base(&self) -> &'static str {
        if self.brand == "lark" {
            LARK_API_BASE
        } else {
            FEISHU_API_BASE
        }
    }
}
