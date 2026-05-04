//! DingTalk API type definitions.
//!
//! Covers: OAuth access token, gateway connection, WebSocket frames, message send, bot config.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// DingTalk API base URL.
pub const DINGTALK_API_BASE: &str = "https://api.dingtalk.com";

/// Bot message callback topic.
pub const TOPIC_ROBOT: &str = "/v1.0/im/bot/messages/get";

/// Token refresh safety margin (refresh 5 minutes before expiry).
pub const TOKEN_REFRESH_AHEAD_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// OAuth Access Token
// ---------------------------------------------------------------------------

/// Request body for `POST /v1.0/oauth2/accessToken`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthTokenRequest {
    pub app_key: String,
    pub app_secret: String,
}

/// Response from OAuth token endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthTokenResponse {
    pub access_token: Option<String>,
    pub expire_in: Option<u64>,
}

// ---------------------------------------------------------------------------
// Gateway Connection (Stream mode)
// ---------------------------------------------------------------------------

/// Subscription entry for gateway open request.
#[derive(Debug, Clone, Serialize)]
pub struct Subscription {
    pub r#type: String,
    pub topic: String,
}

/// Request body for `POST /v1.0/gateway/connections/open`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayOpenRequest {
    pub client_id: String,
    pub client_secret: String,
    pub ua: String,
    pub subscriptions: Vec<Subscription>,
}

/// Response from gateway open endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayOpenResponse {
    pub endpoint: Option<String>,
    pub ticket: Option<String>,
}

// ---------------------------------------------------------------------------
// WebSocket frames (JSON text protocol)
// ---------------------------------------------------------------------------

/// Downstream frame received from DingTalk Stream WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct WsDownStream {
    pub r#type: Option<String>,
    pub headers: Option<WsHeaders>,
    pub data: Option<String>,
    pub spec_version: Option<String>,
    pub message_id: Option<String>,
}

/// Headers in a WebSocket downstream frame.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsHeaders {
    pub content_type: Option<String>,
    pub message_id: Option<String>,
    pub topic: Option<String>,
    pub event_type: Option<String>,
    pub app_id: Option<String>,
    pub connection_id: Option<String>,
    pub time: Option<u64>,
}

/// ACK frame sent back on the WebSocket after receiving a CALLBACK.
#[derive(Debug, Clone, Serialize)]
pub struct WsAck {
    pub code: u32,
    pub headers: WsAckHeaders,
    pub message: String,
    pub data: String,
}

/// Headers in an ACK frame.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsAckHeaders {
    pub message_id: String,
    pub content_type: String,
}

impl WsAck {
    pub fn for_message(message_id: &str) -> Self {
        Self {
            code: 200,
            headers: WsAckHeaders {
                message_id: message_id.to_string(),
                content_type: "application/json".to_string(),
            },
            message: "OK".to_string(),
            data: r#"{"response":{"success":true}}"#.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Inbound message (parsed from CALLBACK data field)
// ---------------------------------------------------------------------------

/// Bot message received from DingTalk Stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkInboundMessage {
    pub sender_id: Option<String>,
    pub sender_nick: Option<String>,
    pub conversation_type: Option<String>,
    pub conversation_id: Option<String>,
    pub msgtype: Option<String>,
    pub text: Option<TextContent>,
    pub content: Option<serde_json::Value>,
    pub at_users: Option<Vec<AtUser>>,
    pub robot_code: Option<String>,
}

/// Text message content.
#[derive(Debug, Clone, Deserialize)]
pub struct TextContent {
    pub content: Option<String>,
}

/// @mention user entry.
#[derive(Debug, Clone, Deserialize)]
pub struct AtUser {
    pub dingtalk_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Outbound message send
// ---------------------------------------------------------------------------

/// Request body for `POST /v1.0/robot/oToMessages/batchSend` (direct message).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendDirectRequest {
    pub robot_code: String,
    pub user_ids: Vec<String>,
    pub msg_key: String,
    pub msg_param: String,
}

/// Request body for `POST /v1.0/robot/groupMessages/send` (group message).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendGroupRequest {
    pub robot_code: String,
    pub open_conversation_id: String,
    pub msg_key: String,
    pub msg_param: String,
}

// ---------------------------------------------------------------------------
// Tenant configuration (read from bot.toml)
// ---------------------------------------------------------------------------

/// Per-tenant configuration parsed from bot.toml.
#[derive(Debug, Clone)]
pub struct DingTalkTenantConfig {
    pub name: String,
    pub app_key: String,
    pub app_secret: String,
}
