//! tenant_access_token management for Feishu/Lark.
//!
//! Fetches and caches the tenant_access_token (2h validity, auto-refresh).
//! Uses POST `/open-apis/auth/v3/tenant_access_token/internal`.

use crate::api;
use crate::types::*;
use reqwest::Client;
use std::sync::Mutex;
use std::time::Instant;
use tracing::info;

/// Cached tenant_access_token with expiry time.
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Thread-safe cache for a single tenant's access token.
pub struct TenantTokenCache {
    app_id: String,
    app_secret: String,
    api_base: String,
    http: Client,
    token: Mutex<Option<CachedToken>>,
}

impl TenantTokenCache {
    pub fn new(app_id: String, app_secret: String, api_base: &str) -> Self {
        Self {
            app_id,
            app_secret,
            api_base: api_base.to_string(),
            http: Client::new(),
            token: Mutex::new(None),
        }
    }

    /// Get a valid tenant_access_token, refreshing if necessary.
    pub fn get_token(&self) -> Result<String, String> {
        // Check cached token
        {
            let guard = self.token.lock().unwrap();
            if let Some(ref cached) = *guard {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Need to refresh — do a blocking HTTP call.
        // Since this is called from a cdylib plugin thread with its own runtime,
        // we create a temporary runtime for the refresh.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create token refresh runtime: {e}"))?;

        rt.block_on(async { self.refresh().await })
    }

    /// Fetch a new tenant_access_token from Feishu API.
    async fn refresh(&self) -> Result<String, String> {
        let resp = api::get_tenant_token(
            &self.http,
            &self.api_base,
            &self.app_id,
            &self.app_secret,
        )
        .await?;

        if resp.code != 0 {
            return Err(format!(
                "Feishu token API error: code={} msg={}",
                resp.code, resp.msg
            ));
        }

        let token = resp
            .tenant_access_token
            .ok_or("Missing tenant_access_token in response")?;
        let expire_secs = resp.expire.unwrap_or(7200);

        // Refresh 5 minutes early
        let expires_at =
            Instant::now() + std::time::Duration::from_secs(expire_secs.saturating_sub(TOKEN_REFRESH_AHEAD_SECS));

        {
            let mut guard = self.token.lock().unwrap();
            *guard = Some(CachedToken {
                access_token: token.clone(),
                expires_at,
            });
        }

        info!(app_id = %self.app_id, expire_secs, "Refreshed Feishu tenant_access_token");
        Ok(token)
    }

    /// Get the HTTP client (for use by api functions).
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Get the API base URL.
    pub fn api_base(&self) -> &str {
        &self.api_base
    }
}
