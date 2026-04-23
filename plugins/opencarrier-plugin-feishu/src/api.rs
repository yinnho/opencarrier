//! Feishu/Lark REST API client.
//!
//! Stateless async functions for: token acquisition, message send/reply, WS endpoint.

use crate::types::*;
use reqwest::{header::HeaderMap, Client};
use std::time::Duration;

/// Build standard Feishu API headers with Bearer token.
fn feishu_headers(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("Content-Type", "application/json".parse().unwrap());
    h.insert("Authorization", format!("Bearer {token}").parse().unwrap());
    h
}

/// POST `/open-apis/auth/v3/tenant_access_token/internal`
///
/// Exchange app_id/app_secret for a tenant_access_token (2h validity).
pub async fn get_tenant_token(
    http: &Client,
    base: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<TenantTokenResponse, String> {
    let url = format!("{base}/open-apis/auth/v3/tenant_access_token/internal");
    let body = TenantTokenRequest {
        app_id: app_id.to_string(),
        app_secret: app_secret.to_string(),
    };

    let resp = http
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Feishu token request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Feishu token HTTP {status}: {body}"));
    }

    resp.json::<TenantTokenResponse>()
        .await
        .map_err(|e| format!("Feishu token parse error: {e}"))
}

/// POST `/open-apis/im/v1/messages`
///
/// Send a message to a chat or user.
pub async fn send_message(
    http: &Client,
    token: &str,
    base: &str,
    receive_id: &str,
    receive_id_type: &str,
    msg_type: &str,
    content: &str,
) -> Result<SendMessageResponse, String> {
    let url = format!("{base}/open-apis/im/v1/messages?receive_id_type={receive_id_type}");
    let body = SendMessageRequest {
        receive_id: receive_id.to_string(),
        msg_type: msg_type.to_string(),
        content: content.to_string(),
    };

    let resp = http
        .post(&url)
        .headers(feishu_headers(token))
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Feishu send_message request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Feishu send_message HTTP {status}: {body}"));
    }

    resp.json::<SendMessageResponse>()
        .await
        .map_err(|e| format!("Feishu send_message parse error: {e}"))
}

/// POST `/open-apis/im/v1/messages/{message_id}/reply`
///
/// Reply to a specific message.
pub async fn reply_message(
    http: &Client,
    token: &str,
    base: &str,
    message_id: &str,
    msg_type: &str,
    content: &str,
) -> Result<SendMessageResponse, String> {
    let url = format!("{base}/open-apis/im/v1/messages/{message_id}/reply");
    let body = ReplyMessageRequest {
        content: content.to_string(),
        msg_type: msg_type.to_string(),
    };

    let resp = http
        .post(&url)
        .headers(feishu_headers(token))
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Feishu reply_message request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Feishu reply_message HTTP {status}: {body}"));
    }

    resp.json::<SendMessageResponse>()
        .await
        .map_err(|e| format!("Feishu reply_message parse error: {e}"))
}

/// POST `/open-apis/callback/ws/endpoint`
///
/// Get the WebSocket URL for long-connection event subscription.
pub async fn get_ws_endpoint(
    http: &Client,
    token: &str,
    base: &str,
) -> Result<WsEndpointResponse, String> {
    let url = format!("{base}/open-apis/callback/ws/endpoint");

    let resp = http
        .post(&url)
        .headers(feishu_headers(token))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Feishu ws/endpoint request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Feishu ws/endpoint HTTP {status}: {body}"));
    }

    resp.json::<WsEndpointResponse>()
        .await
        .map_err(|e| format!("Feishu ws/endpoint parse error: {e}"))
}
