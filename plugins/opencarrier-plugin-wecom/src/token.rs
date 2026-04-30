//! Token management and WeCom API helpers.
//!
//! Supports three WeCom integration modes:
//! - **App** (企业应用): corp_id + agent_id + secret → message/send
//! - **Kf** (微信客服): corp_id + open_kfid + secret → kf/send_msg
//! - **SmartBot** (智能对话机器人): WebSocket long connection + response_url reply

use dashmap::DashMap;
use reqwest::Client;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::info;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const WECOM_API_BASE: &str = "https://qyapi.weixin.qq.com";

/// Refresh token 5 minutes before actual expiry.
const TOKEN_REFRESH_BUFFER_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// WeCom integration mode
// ---------------------------------------------------------------------------

/// Which WeCom API to use for this tenant.
pub enum WecomMode {
    /// Enterprise application — sends via `cgi-bin/message/send`.
    App { agent_id: String },
    /// Customer service — sends via `cgi-bin/kf/send_msg`.
    Kf { open_kfid: String },
    /// Smart dialog bot — WebSocket long connection to `wss://openws.work.weixin.qq.com`.
    SmartBot { bot_id: String, secret: String },
}

// ---------------------------------------------------------------------------
// Tenant entry
// ---------------------------------------------------------------------------

/// Per-tenant configuration and cached token.
pub struct TenantEntry {
    /// Unique tenant name (used as DashMap key).
    pub name: String,
    /// Enterprise corp ID (not used for Bot mode).
    pub corp_id: String,
    /// Application/customer-service secret (not used for Bot mode).
    pub secret: String,
    /// Webhook port for callback server (App and Kf modes).
    pub webhook_port: u16,
    /// AES key for callback encryption (App and Kf modes).
    pub encoding_aes_key: Option<String>,
    /// Callback verification token (App and Kf modes).
    pub callback_token: Option<String>,
    /// Integration mode.
    pub mode: WecomMode,
    /// Shared HTTP client.
    pub http: Client,
    /// Cached access token with expiry.
    cached_token: Mutex<Option<(String, Instant)>>,
    /// MCP bot credentials (for App/Kf modes; SmartBot reuses mode's bot_id/secret).
    pub mcp_bot_id: Option<String>,
    pub mcp_bot_secret: Option<String>,
}

impl TenantEntry {
    // -----------------------------------------------------------------------
    // Constructors per mode
    // -----------------------------------------------------------------------

    /// Create an enterprise application tenant.
    #[allow(clippy::too_many_arguments)]
    pub fn new_app(
        name: String,
        corp_id: String,
        agent_id: String,
        secret: String,
        webhook_port: u16,
        encoding_aes_key: Option<String>,
        callback_token: Option<String>,
        mcp_bot_id: Option<String>,
        mcp_bot_secret: Option<String>,
    ) -> Self {
        Self {
            name,
            corp_id,
            secret,
            webhook_port,
            encoding_aes_key,
            callback_token,
            mode: WecomMode::App { agent_id },
            http: Client::new(),
            cached_token: Mutex::new(None),
            mcp_bot_id,
            mcp_bot_secret,
        }
    }

    /// Create a customer service tenant.
    #[allow(clippy::too_many_arguments)]
    pub fn new_kf(
        name: String,
        corp_id: String,
        open_kfid: String,
        secret: String,
        webhook_port: u16,
        encoding_aes_key: Option<String>,
        callback_token: Option<String>,
        mcp_bot_id: Option<String>,
        mcp_bot_secret: Option<String>,
    ) -> Self {
        Self {
            name,
            corp_id,
            secret,
            webhook_port,
            encoding_aes_key,
            callback_token,
            mode: WecomMode::Kf { open_kfid },
            http: Client::new(),
            cached_token: Mutex::new(None),
            mcp_bot_id,
            mcp_bot_secret,
        }
    }

    /// Create a smart dialog bot tenant.
    pub fn new_smartbot(name: String, corp_id: String, bot_id: String, secret: String) -> Self {
        Self {
            name,
            corp_id,
            secret: secret.clone(),
            webhook_port: 0,
            encoding_aes_key: None,
            callback_token: None,
            mode: WecomMode::SmartBot { bot_id, secret },
            http: Client::new(),
            cached_token: Mutex::new(None),
            mcp_bot_id: None, // SmartBot uses mode's bot_id directly
            mcp_bot_secret: None,
        }
    }

    // -----------------------------------------------------------------------
    // Access helpers
    // -----------------------------------------------------------------------

    /// Get agent_id if this is an App-mode tenant.
    pub fn agent_id(&self) -> Option<&str> {
        match &self.mode {
            WecomMode::App { agent_id } => Some(agent_id),
            _ => None,
        }
    }

    /// Get open_kfid if this is a Kf-mode tenant.
    pub fn open_kfid(&self) -> Option<&str> {
        match &self.mode {
            WecomMode::Kf { open_kfid } => Some(open_kfid),
            _ => None,
        }
    }

    /// Get bot_id if this is a SmartBot-mode tenant.
    pub fn bot_id(&self) -> Option<&str> {
        match &self.mode {
            WecomMode::SmartBot { bot_id, .. } => Some(bot_id),
            _ => None,
        }
    }

    /// Get bot secret if this is a SmartBot-mode tenant.
    pub fn bot_secret(&self) -> Option<&str> {
        match &self.mode {
            WecomMode::SmartBot { secret, .. } => Some(secret),
            _ => None,
        }
    }

    /// Get MCP bot credentials (bot_id, bot_secret).
    /// SmartBot mode reuses its mode's bot_id and secret.
    /// App/Kf modes use the dedicated mcp_bot_id/mcp_bot_secret fields.
    pub fn mcp_credentials(&self) -> Option<(&str, &str)> {
        match &self.mode {
            WecomMode::SmartBot { bot_id, secret } => Some((bot_id, secret)),
            _ => self
                .mcp_bot_id
                .as_deref()
                .zip(self.mcp_bot_secret.as_deref()),
        }
    }

    /// Get a valid access token, refreshing if needed.
    /// Returns error for SmartBot mode (no token needed).
    pub fn get_access_token(&self) -> Result<String, String> {
        match &self.mode {
            WecomMode::SmartBot { .. } => Err("SmartBot mode does not use access tokens".into()),
            _ => self.get_or_refresh_token(),
        }
    }

    fn get_or_refresh_token(&self) -> Result<String, String> {
        // Check cache
        if let Some((token, expires_at)) = self.cached_token.lock().unwrap().as_ref() {
            if Instant::now() < *expires_at {
                return Ok(token.clone());
            }
        }

        // Fetch new token
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Runtime error: {e}"))?;
        let token = rt.block_on(self.fetch_token())?;

        Ok(token)
    }

    async fn fetch_token(&self) -> Result<String, String> {
        let url = format!(
            "{}/cgi-bin/gettoken?corpid={}&corpsecret={}",
            WECOM_API_BASE, self.corp_id, self.secret
        );

        let resp: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("token request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("token response parse error: {e}"))?;

        let errcode = resp["errcode"].as_i64().unwrap_or(-1);
        if errcode != 0 {
            let errmsg = resp["errmsg"].as_str().unwrap_or("unknown");
            return Err(format!("token error: {errcode} {errmsg}"));
        }

        let token = resp["access_token"]
            .as_str()
            .ok_or("missing access_token")?
            .to_string();
        let expires_in = resp["expires_in"].as_u64().unwrap_or(7200);

        let expires_at =
            Instant::now() + Duration::from_secs(expires_in.saturating_sub(TOKEN_REFRESH_BUFFER_SECS));

        info!(tenant = %self.name, "Refreshed WeCom access token");

        *self.cached_token.lock().unwrap() = Some((token.clone(), expires_at));
        Ok(token)
    }
}

// ---------------------------------------------------------------------------
// Token manager
// ---------------------------------------------------------------------------

/// Multi-tenant token manager keyed by tenant name.
pub struct TokenManager {
    pub tenants: DashMap<String, TenantEntry>,
}

impl TokenManager {
    pub fn new() -> Self {
        Self {
            tenants: DashMap::new(),
        }
    }

    /// Add a tenant.
    pub fn add_tenant(&self, entry: TenantEntry) {
        let name = entry.name.clone();
        self.tenants.insert(name, entry);
    }

    /// Get a tenant entry by name.
    pub fn get_tenant(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, String, TenantEntry>> {
        self.tenants.get(name)
    }

    /// Get all tenant names.
    pub fn tenant_names(&self) -> Vec<String> {
        self.tenants.iter().map(|e| e.key().clone()).collect()
    }

    /// Get access token for a tenant.
    #[allow(dead_code)]
    pub fn get_access_token(&self, name: &str) -> Result<String, String> {
        let entry = self
            .tenants
            .get(name)
            .ok_or_else(|| format!("Unknown tenant: {name}"))?;
        entry.get_access_token()
    }
}

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

/// Make a POST request to a WeCom API endpoint (with access_token).
pub async fn wedoc_post(
    http: &Client,
    path: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}?access_token={}", WECOM_API_BASE, path, token);
    let resp: serde_json::Value = http
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("API request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("API response parse error: {e}"))?;

    let errcode = resp["errcode"].as_i64().unwrap_or(-1);
    if errcode != 0 {
        let errmsg = resp["errmsg"].as_str().unwrap_or("unknown");
        return Err(format!("WeCom API error {errcode}: {errmsg}"));
    }

    Ok(resp)
}

/// Send an application message to a WeCom user (App mode).
pub fn send_app_message(
    tenant: &TenantEntry,
    user_id: &str,
    content: &str,
) -> Result<(), String> {
    let agent_id = tenant
        .agent_id()
        .ok_or("send_app_message requires App mode")?
        .to_string();
    let token = tenant.get_access_token()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime error: {e}"))?;
    rt.block_on(async {
        let body = serde_json::json!({
            "touser": user_id,
            "msgtype": "text",
            "agentid": agent_id,
            "text": { "content": content }
        });
        wedoc_post(&tenant.http, "cgi-bin/message/send", &token, &body).await
    })?;

    Ok(())
}

/// Send a customer service message (Kf mode).
pub fn send_kf_message(
    tenant: &TenantEntry,
    user_id: &str,
    content: &str,
) -> Result<(), String> {
    let open_kfid = tenant
        .open_kfid()
        .ok_or("send_kf_message requires Kf mode")?
        .to_string();
    let token = tenant.get_access_token()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime error: {e}"))?;
    rt.block_on(async {
        let body = serde_json::json!({
            "touser": user_id,
            "open_kfid": open_kfid,
            "msgtype": "text",
            "text": { "content": content }
        });
        wedoc_post(&tenant.http, "cgi-bin/kf/send_msg", &token, &body).await
    })?;

    Ok(())
}

/// Send a reply via the SmartBot response_url (HTTP POST with markdown).
#[allow(dead_code)]
pub fn send_smartbot_response(http: &Client, response_url: &str, content: &str) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime error: {e}"))?;
    rt.block_on(send_smartbot_response_async(http, response_url, content))
}

/// Async version of send_smartbot_response (for use within plugin's own runtime).
pub async fn send_smartbot_response_async(http: &Client, response_url: &str, content: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "content": content
        }
    });
    let resp: serde_json::Value = http
        .post(response_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("smartbot response failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("smartbot response parse error: {e}"))?;

    let errcode = resp["errcode"].as_i64().unwrap_or(-1);
    if errcode != 0 {
        let errmsg = resp["errmsg"].as_str().unwrap_or("unknown");
        return Err(format!("smartbot response error {errcode}: {errmsg}"));
    }

    Ok(())
}
