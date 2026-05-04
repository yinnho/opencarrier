//! DingTalk OAuth access token management.
//!
//! Fetches and caches the accessToken (with 5-minute early refresh).
//! Uses POST `/v1.0/oauth2/accessToken`.

use crate::plugin::channels::dingtalk::api;
use crate::plugin::channels::dingtalk::types::*;
use reqwest::Client;
use std::sync::Mutex;
use std::time::Instant;
use tracing::info;

/// Cached access token with expiry time.
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Thread-safe cache for a DingTalk app's access token.
pub struct AccessTokenCache {
    app_key: String,
    app_secret: String,
    http: Client,
    token: Mutex<Option<CachedToken>>,
}

impl AccessTokenCache {
    pub fn new(app_key: String, app_secret: String) -> Self {
        Self {
            app_key,
            app_secret,
            http: Client::new(),
            token: Mutex::new(None),
        }
    }

    /// Get a valid access token, refreshing if necessary.
    pub async fn get_token(&self) -> Result<String, String> {
        {
            let guard = self.token.lock().unwrap();
            if let Some(ref cached) = *guard {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        self.refresh().await
    }

    /// Fetch a new access token from DingTalk OAuth API.
    async fn refresh(&self) -> Result<String, String> {
        let resp = api::get_access_token(&self.http, &self.app_key, &self.app_secret).await?;

        let token = resp
            .access_token
            .ok_or("Missing accessToken in DingTalk OAuth response")?;
        let expire_secs = resp.expire_in.unwrap_or(7200);

        let expires_at = Instant::now()
            + std::time::Duration::from_secs(expire_secs.saturating_sub(TOKEN_REFRESH_AHEAD_SECS));

        {
            let mut guard = self.token.lock().unwrap();
            *guard = Some(CachedToken {
                access_token: token.clone(),
                expires_at,
            });
        }

        info!(app_key = %self.app_key, expire_secs, "Refreshed DingTalk access token");
        Ok(token)
    }

    pub fn http(&self) -> &Client {
        &self.http
    }

    pub fn app_key(&self) -> &str {
        &self.app_key
    }

    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }
}
