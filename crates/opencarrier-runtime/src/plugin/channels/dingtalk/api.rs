//! DingTalk REST API client.
//!
//! Stateless async functions for: OAuth token, gateway open, message send.

use crate::plugin::channels::dingtalk::types::*;
use reqwest::{header::HeaderMap, Client};
use std::time::Duration;

fn dingtalk_headers(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("Content-Type", "application/json".parse().unwrap());
    h.insert(
        "x-acs-dingtalk-access-token",
        token.parse().unwrap(),
    );
    h
}

/// POST `/v1.0/oauth2/accessToken`
///
/// Exchange app_key/app_secret for an access token.
pub async fn get_access_token(
    http: &Client,
    app_key: &str,
    app_secret: &str,
) -> Result<OAuthTokenResponse, String> {
    let url = format!("{DINGTALK_API_BASE}/v1.0/oauth2/accessToken");
    let body = OAuthTokenRequest {
        app_key: app_key.to_string(),
        app_secret: app_secret.to_string(),
    };

    let resp = http
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("DingTalk token request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("DingTalk token HTTP {status}: {body}"));
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| format!("DingTalk token parse error: {e}"))
}

/// POST `/v1.0/gateway/connections/open`
///
/// Open a Stream gateway connection and get the WebSocket endpoint + ticket.
/// Note: This endpoint does NOT use the access token header — authentication
/// is via clientId/clientSecret in the request body (matching the TS SDK behavior).
pub async fn open_gateway(
    http: &Client,
    client_id: &str,
    client_secret: &str,
) -> Result<GatewayOpenResponse, String> {
    let url = format!("{DINGTALK_API_BASE}/v1.0/gateway/connections/open");
    let body = GatewayOpenRequest {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        ua: "opencarrier".to_string(),
        subscriptions: vec![
            Subscription {
                r#type: "EVENT".to_string(),
                topic: "*".to_string(),
            },
            Subscription {
                r#type: "CALLBACK".to_string(),
                topic: TOPIC_ROBOT.to_string(),
            },
        ],
    };

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Accept", "application/json".parse().unwrap());

    let resp = http
        .post(&url)
        .headers(headers)
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("DingTalk gateway open failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("DingTalk gateway open HTTP {status}: {body}"));
    }

    resp.json::<GatewayOpenResponse>()
        .await
        .map_err(|e| format!("DingTalk gateway open parse error: {e}"))
}

/// POST `/v1.0/robot/oToMessages/batchSend`
///
/// Send a markdown message to a user (direct message).
pub async fn send_direct_message(
    http: &Client,
    token: &str,
    robot_code: &str,
    user_id: &str,
    text: &str,
) -> Result<(), String> {
    let url = format!("{DINGTALK_API_BASE}/v1.0/robot/oToMessages/batchSend");
    let title = text.lines().next().unwrap_or("Reply").to_string();
    let title = title.chars().take(20).collect::<String>();

    let body = SendDirectRequest {
        robot_code: robot_code.to_string(),
        user_ids: vec![user_id.to_string()],
        msg_key: "sampleMarkdown".to_string(),
        msg_param: serde_json::json!({ "title": title, "text": text }).to_string(),
    };

    let resp = http
        .post(&url)
        .headers(dingtalk_headers(token))
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("DingTalk send_direct failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("DingTalk send_direct HTTP {status}: {body}"));
    }

    Ok(())
}

/// POST `/v1.0/robot/groupMessages/send`
///
/// Send a markdown message to a group.
pub async fn send_group_message(
    http: &Client,
    token: &str,
    robot_code: &str,
    conversation_id: &str,
    text: &str,
) -> Result<(), String> {
    let url = format!("{DINGTALK_API_BASE}/v1.0/robot/groupMessages/send");
    let title = text.lines().next().unwrap_or("Reply").to_string();
    let title = title.chars().take(20).collect::<String>();

    let body = SendGroupRequest {
        robot_code: robot_code.to_string(),
        open_conversation_id: conversation_id.to_string(),
        msg_key: "sampleMarkdown".to_string(),
        msg_param: serde_json::json!({ "title": title, "text": text }).to_string(),
    };

    let resp = http
        .post(&url)
        .headers(dingtalk_headers(token))
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("DingTalk send_group failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("DingTalk send_group HTTP {status}: {body}"));
    }

    Ok(())
}
