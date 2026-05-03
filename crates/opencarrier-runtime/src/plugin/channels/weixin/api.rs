//! iLink Bot API HTTP client.
//!
//! Stateless async functions wrapping all iLink endpoints at `ilinkai.weixin.qq.com`.

use base64::Engine;
use rand::Rng;
use reqwest::{header::HeaderMap, Client};
use std::time::Duration;

use crate::plugin::channels::weixin::types::*;

/// Build the required iLink request headers (with optional Bearer token).
fn ilink_headers(bot_token: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("Content-Type", "application/json".parse().unwrap());
    h.insert("AuthorizationType", "ilink_bot_token".parse().unwrap());
    h.insert("iLink-App-Id", "bot".parse().unwrap());

    // Client version: (major << 16) | (minor << 8) | patch
    let client_ver = ((1u32 << 16) | 2).to_string();
    h.insert("iLink-App-ClientVersion", client_ver.parse().unwrap());

    // X-WECHAT-UIN: random uint32 -> decimal string -> base64
    let uin = rand::thread_rng().gen::<u32>();
    let encoded = base64::engine::general_purpose::STANDARD.encode(uin.to_string());
    h.insert("X-WECHAT-UIN", encoded.parse().unwrap());

    if let Some(token) = bot_token {
        if !token.is_empty() {
            h.insert("Authorization", format!("Bearer {token}").parse().unwrap());
        }
    }

    h
}

/// GET `/ilink/bot/get_bot_qrcode?bot_type=3`
///
/// No auth required. Returns QR code for WeChat scanning.
pub async fn get_bot_qrcode(http: &Client) -> Result<QrCodeResponse, String> {
    get_bot_qrcode_with_base(http, ILINK_API_BASE).await
}

/// GET `<base>/ilink/bot/get_bot_qrcode?bot_type=3` with custom base URL.
pub async fn get_bot_qrcode_with_base(
    http: &Client,
    base_url: &str,
) -> Result<QrCodeResponse, String> {
    let url = format!("{base_url}/ilink/bot/get_bot_qrcode?bot_type={BOT_TYPE}");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("get_bot_qrcode request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("get_bot_qrcode HTTP {status}: {body}"));
    }

    resp.json::<QrCodeResponse>()
        .await
        .map_err(|e| format!("get_bot_qrcode parse error: {e}"))
}

/// GET `<base>/ilink/bot/get_qrcode_status?qrcode=xxx`
///
/// No auth required. Long-polls for scan status (server holds up to 35s).
pub async fn get_qrcode_status(
    http: &Client,
    base_url: &str,
    qrcode: &str,
) -> Result<QrCodeStatusResponse, String> {
    let url = format!(
        "{base_url}/ilink/bot/get_qrcode_status?qrcode={}",
        urlencoding::encode(qrcode)
    );
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .map_err(|e| format!("get_qrcode_status request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("get_qrcode_status HTTP {status}: {body}"));
    }

    // iLink may return application/octet-stream content type
    let text = resp
        .text()
        .await
        .map_err(|e| format!("get_qrcode_status read body error: {e}"))?;

    serde_json::from_str::<QrCodeStatusResponse>(&text)
        .map_err(|e| format!("get_qrcode_status parse error: {e}: {text}"))
}

/// POST `/ilink/bot/getupdates`
///
/// Long-poll receive messages. Server holds up to 35s.
pub async fn get_updates(
    http: &Client,
    bot_token: &str,
    baseurl: &str,
    cursor: &str,
) -> Result<GetUpdatesResponse, String> {
    let url = format!("{baseurl}/ilink/bot/getupdates");
    let body = GetUpdatesRequest {
        get_updates_buf: cursor.to_string(),
        base_info: BaseInfo::default(),
    };

    let resp = http
        .post(&url)
        .headers(ilink_headers(Some(bot_token)))
        .json(&body)
        .timeout(Duration::from_millis(LONG_POLL_TIMEOUT_MS + 5_000))
        .send()
        .await
        .map_err(|e| format!("getupdates request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("getupdates HTTP {status}: {body}"));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("getupdates read body error: {e}"))?;

    serde_json::from_str::<GetUpdatesResponse>(&text)
        .map_err(|e| format!("getupdates parse error: {e}"))
}

/// POST `/ilink/bot/sendmessage`
///
/// Send a message to a WeChat user. Requires context_token from an inbound message.
pub async fn send_message(
    http: &Client,
    bot_token: &str,
    baseurl: &str,
    to_user_id: &str,
    context_token: &str,
    client_id: &str,
    text: &str,
) -> Result<(), String> {
    let url = format!("{baseurl}/ilink/bot/sendmessage");

    let req = SendMessageRequest {
        msg: SendMessageMsg {
            from_user_id: String::new(),
            to_user_id: to_user_id.to_string(),
            client_id: client_id.to_string(),
            message_type: MSG_TYPE_BOT,
            message_state: MSG_STATE_FINISH,
            context_token: Some(context_token.to_string()),
            item_list: Some(vec![SendItem {
                type_: ITEM_TYPE_TEXT,
                text_item: Some(SendTextItem {
                    text: text.to_string(),
                }),
            }]),
        },
        base_info: BaseInfo::default(),
    };

    let resp = http
        .post(&url)
        .headers(ilink_headers(Some(bot_token)))
        .json(&req)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("sendmessage request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("sendmessage HTTP {status}: {body}"));
    }

    // iLink returns empty JSON or { } on success
    let _ = resp
        .text()
        .await
        .map_err(|e| format!("sendmessage read body error: {e}"))?;

    Ok(())
}

/// POST `/ilink/bot/getconfig`
///
/// Get typing_ticket for a user (cached for 24h).
pub async fn get_config(
    http: &Client,
    bot_token: &str,
    baseurl: &str,
    ilink_user_id: &str,
    context_token: Option<&str>,
) -> Result<GetConfigResponse, String> {
    let url = format!("{baseurl}/ilink/bot/getconfig");

    let body = GetConfigRequest {
        ilink_user_id: ilink_user_id.to_string(),
        context_token: context_token.map(|s| s.to_string()),
        base_info: BaseInfo::default(),
    };

    let resp = http
        .post(&url)
        .headers(ilink_headers(Some(bot_token)))
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("getconfig request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("getconfig HTTP {status}: {body}"));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("getconfig read body error: {e}"))?;

    serde_json::from_str::<GetConfigResponse>(&text)
        .map_err(|e| format!("getconfig parse error: {e}"))
}

/// POST `/ilink/bot/sendtyping`
///
/// Send typing indicator (status=1 for typing, status=2 for cancel).
pub async fn send_typing(
    http: &Client,
    bot_token: &str,
    baseurl: &str,
    ilink_user_id: &str,
    typing_ticket: &str,
    status: u32,
) -> Result<(), String> {
    let url = format!("{baseurl}/ilink/bot/sendtyping");

    let body = SendTypingRequest {
        ilink_user_id: ilink_user_id.to_string(),
        typing_ticket: typing_ticket.to_string(),
        status,
        base_info: BaseInfo::default(),
    };

    let resp = http
        .post(&url)
        .headers(ilink_headers(Some(bot_token)))
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("sendtyping request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("sendtyping HTTP {status}: {body}"));
    }

    Ok(())
}
