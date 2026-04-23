//! WeChat Official Account API client.
//!
//! Handles access_token lifecycle (fetch + cache + auto-refresh) and
//! provides thin wrappers around the WeChat REST endpoints.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio::sync::Mutex;

const WECHAT_API_BASE: &str = "https://api.weixin.qq.com";

/// Refresh the token this many seconds before it actually expires.
const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WeChatClient {
    app_id: String,
    app_secret: String,
    http: reqwest::Client,
    token: Arc<Mutex<Option<CachedToken>>>,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

// ---------------------------------------------------------------------------
// WeChat JSON response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

impl WeChatClient {
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self {
            app_id,
            app_secret,
            http: reqwest::Client::new(),
            token: Arc::new(Mutex::new(None)),
        }
    }

    /// Obtain a valid access_token, refreshing from WeChat when needed.
    pub async fn get_token(&self) -> Result<String> {
        // Fast path — cached and not about to expire.
        {
            let guard = self.token.lock().await;
            if let Some(cached) = guard.as_ref() {
                if cached.expires_at > Instant::now() + TOKEN_EXPIRY_MARGIN {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Slow path — hit the WeChat API.
        let url = format!(
            "{}/cgi-bin/token?grant_type=client_credential&appid={}&secret={}",
            WECHAT_API_BASE, self.app_id, self.app_secret
        );
        let resp: TokenResponse = self.http.get(&url).send().await?.json().await?;

        if let Some(code) = resp.errcode {
            if code != 0 {
                bail!(
                    "WeChat token error {}: {}",
                    code,
                    resp.errmsg.unwrap_or_default()
                );
            }
        }

        let access_token = resp.access_token.context("no access_token in response")?;
        let expires_in = resp.expires_in.unwrap_or(7200);

        {
            let mut guard = self.token.lock().await;
            *guard = Some(CachedToken {
                access_token: access_token.clone(),
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        Ok(access_token)
    }

    /// POST JSON body with auto-injected access_token.
    pub async fn api_post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let token = self.get_token().await?;
        let url = format!("{}{}?access_token={}", WECHAT_API_BASE, path, token);
        let json: serde_json::Value = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await?
            .json()
            .await?;
        check_error(&json)?;
        Ok(json)
    }

    /// Upload binary media via multipart/form-data.
    pub async fn upload_media(
        &self,
        media_type: &str,
        filename: &str,
        data: &[u8],
    ) -> Result<serde_json::Value> {
        let token = self.get_token().await?;
        let url = format!(
            "{}{}?access_token={}&type={}",
            WECHAT_API_BASE, "/cgi-bin/material/add_material", token, media_type
        );

        let mime = match media_type {
            "image" => "image/jpeg",
            "voice" => "audio/mpeg",
            "video" => "video/mp4",
            _ => "application/octet-stream",
        };
        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str(mime)?;
        let form = reqwest::multipart::Form::new().part("media", part);

        let json: serde_json::Value = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;
        check_error(&json)?;
        Ok(json)
    }
}

/// Check for `errcode != 0` in a WeChat JSON response.
fn check_error(json: &serde_json::Value) -> Result<()> {
    if let Some(code) = json.get("errcode").and_then(|v| v.as_i64()) {
        if code != 0 {
            let msg = json
                .get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            bail!("WeChat API error {}: {}", code, msg);
        }
    }
    Ok(())
}
