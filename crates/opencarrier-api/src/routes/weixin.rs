//! WeChat iLink Bot, WeCom, and Feishu channel endpoints.

use crate::routes::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use fs4::fs_std::FileExt;
use std::collections::HashMap;
use std::sync::Arc;
/// GET `/api/weixin/qrcode` — fetch a fresh QR code for WeChat scanning.
///
/// Query params: `?tenant=<name>` (optional, defaults to "default")
pub async fn weixin_qrcode(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let raw_tenant = params.get("tenant").map(|s| s.as_str()).unwrap_or("default");
    let tenant = match weixin_sanitize_tenant(raw_tenant) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
            );
        }
    };

    let url = format!(
        "{WEIXIN_ILINK_BASE}/ilink/bot/get_bot_qrcode?bot_type={WEIXIN_BOT_TYPE}"
    );

    let http = weixin_http_client();
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(tenant, "get_bot_qrcode request failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("iLink request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(tenant, %status, "get_bot_qrcode returned {status}: {body}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("iLink HTTP {status}") })),
        );
    }

    match resp.json::<serde_json::Value>().await {
        Ok(data) => (StatusCode::OK, Json(serde_json::json!({
            "tenant": tenant,
            "data": data,
        }))),
        Err(e) => {
            tracing::error!(tenant, "get_bot_qrcode parse error: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            )
        }
    }
}
/// GET `/api/weixin/qrcode-status` — poll QR code scan status.
///
/// Query params: `?tenant=<name>&qrcode=<code>`
///
/// When status becomes "confirmed", saves the bot_token and registers the tenant.
pub async fn weixin_qrcode_status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let raw_tenant = params.get("tenant").map(|s| s.as_str()).unwrap_or("default");
    let tenant = match weixin_sanitize_tenant(raw_tenant) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid tenant name" })),
            );
        }
    };
    let qrcode = match params.get("qrcode") {
        Some(q) => q.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing qrcode parameter" })),
            );
        }
    };

    let url = format!(
        "{WEIXIN_ILINK_BASE}/ilink/bot/get_qrcode_status?qrcode={}",
        urlencoding::encode(&qrcode)
    );

    let http = weixin_http_client();
    let resp = match http
        .get(&url)
        .timeout(std::time::Duration::from_secs(40))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status request failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("iLink request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(tenant, %status, "get_qrcode_status returned {status}: {body}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("iLink HTTP {status}") })),
        );
    }

    // iLink may return application/octet-stream
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status read body error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Read error: {e}") })),
            );
        }
    };

    let data: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status parse error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            );
        }
    };

    // Check if scan is confirmed — if so, extract bot_token and register tenant
    let scan_status = data
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if scan_status == "confirmed" {
        let bot_token = data
            .get("bot_token")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let raw_baseurl = data
            .get("baseurl")
            .and_then(|v| v.as_str())
            .unwrap_or(WEIXIN_ILINK_BASE);
        let ilink_bot_id = data
            .get("ilink_bot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ilink_user_id = data.get("ilink_user_id").and_then(|v| v.as_str());

        // Validate baseurl to prevent stored SSRF
        let baseurl = if weixin_validate_baseurl(raw_baseurl) {
            raw_baseurl
        } else {
            tracing::warn!(tenant, raw_baseurl, "iLink returned unexpected baseurl, falling back to default");
            WEIXIN_ILINK_BASE
        };

        if !bot_token.is_empty() && !ilink_bot_id.is_empty() {
            // Save token to disk so plugin can pick it up on restart
            let token_dir = state.kernel.config.home_dir.join("weixin-tokens");
            if let Err(e) = std::fs::create_dir_all(&token_dir) {
                tracing::error!(tenant, "Failed to create weixin token dir: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to create token directory: {e}") })),
                );
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let token_file = serde_json::json!({
                "name": tenant,
                "bot_token": bot_token,
                "baseurl": baseurl,
                "ilink_bot_id": ilink_bot_id,
                "user_id": ilink_user_id,
                "expires_at": now + 86400, // 24h
                "bind_agent": null,
            });
            let path = token_dir.join(format!("{tenant}.json"));
            match serde_json::to_string_pretty(&token_file) {
                Ok(json) => {
                    if let Err(e) = atomic_write(&path, &json) {
                        tracing::error!(tenant, "Failed to write weixin token file: {e}");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("Failed to save token: {e}") })),
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(tenant, "Failed to serialize token file: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": "Internal serialization error" })),
                    );
                }
            }

            tracing::info!(
                tenant,
                ilink_bot_id,
                "WeChat iLink QR scan confirmed — token saved"
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tenant": tenant,
            "status": scan_status,
            "data": data,
        })),
    )
}
/// GET `/api/weixin/status` — list all bound WeChat accounts with expiry info.
pub async fn weixin_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let token_dir = state.kernel.config.home_dir.join("weixin-tokens");

    let mut tenants: Vec<serde_json::Value> = Vec::new();

    if token_dir.exists() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(entries) = std::fs::read_dir(&token_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                        let expires_at = tf
                            .get("expires_at")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let expired = now >= expires_at;
                        let remaining = (expires_at - now).max(0);

                        tenants.push(serde_json::json!({
                            "name": tf.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            "ilink_bot_id": tf.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or(""),
                            "user_id": tf.get("user_id").and_then(|v| v.as_str()),
                            "expires_at": expires_at,
                            "remaining_secs": remaining,
                            "expired": expired,
                            "bind_agent": tf.get("bind_agent").and_then(|v| v.as_str()),
                        }));
                    }
                }
            }
        }
    }

    Json(serde_json::json!({
        "tenants": tenants,
        "count": tenants.len(),
    }))
}
// ---------------------------------------------------------------------------
// Channels — unified status + tenant management
// ---------------------------------------------------------------------------

/// GET `/api/channels/status` — aggregate status for all channel plugins.
///
/// Reads WeChat token files, WeCom and Feishu plugin.toml tenants.
pub async fn channels_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;

    // ── WeChat iLink ──────────────────────────────────────────────────
    let weixin_dir = home.join("weixin-tokens");
    let mut weixin_tenants: Vec<serde_json::Value> = Vec::new();

    if weixin_dir.exists() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(entries) = std::fs::read_dir(&weixin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                        let expires_at = tf.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
                        let expired = now >= expires_at;
                        let remaining = (expires_at - now).max(0);
                        weixin_tenants.push(serde_json::json!({
                            "name": tf.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            "ilink_bot_id": tf.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or(""),
                            "expired": expired,
                            "remaining_secs": remaining,
                        }));
                    }
                }
            }
        }
    }

    // ── WeCom ──────────────────────────────────────────────────────────
    let wecom_toml = home.join("plugins").join("opencarrier-plugin-wecom").join("plugin.toml");
    let mut wecom_tenants: Vec<serde_json::Value> = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&wecom_toml) {
        if let Ok(doc) = content.parse::<toml::Value>() {
            if let Some(arr) = doc.get("tenants").and_then(|v| v.as_array()) {
                for tenant in arr {
                    let cfg = tenant.get("config").cloned().unwrap_or(toml::Value::Table(Default::default()));
                    wecom_tenants.push(serde_json::json!({
                        "name": tenant.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "mode": cfg.get("mode").and_then(|v| v.as_str()).unwrap_or("smartbot"),
                        "corp_id": cfg.get("corp_id").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
            }
        }
    }

    // ── Feishu/Lark ────────────────────────────────────────────────────
    let feishu_toml = home.join("plugins").join("opencarrier-plugin-feishu").join("plugin.toml");
    let mut feishu_tenants: Vec<serde_json::Value> = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&feishu_toml) {
        if let Ok(doc) = content.parse::<toml::Value>() {
            if let Some(arr) = doc.get("tenants").and_then(|v| v.as_array()) {
                for tenant in arr {
                    let cfg = tenant.get("config").cloned().unwrap_or(toml::Value::Table(Default::default()));
                    feishu_tenants.push(serde_json::json!({
                        "name": tenant.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "app_id": cfg.get("app_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "brand": cfg.get("brand").and_then(|v| v.as_str()).unwrap_or("feishu"),
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "weixin": { "tenants": weixin_tenants, "count": weixin_tenants.len() },
        "wecom": { "tenants": wecom_tenants, "count": wecom_tenants.len() },
        "feishu": { "tenants": feishu_tenants, "count": feishu_tenants.len() },
    }))
}

/// Sanitize tenant name for plugin.toml entries.
fn channel_sanitize_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return None;
    }
    if trimmed.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        Some(trimmed.to_string())
    } else {
        None
    }
}
/// POST `/api/channels/wecom/tenants` — add a WeCom tenant to plugin.toml.
///
/// Body: `{ "name": "...", "mode": "smartbot"|"app"|"kf", "corp_id": "...", "bot_id": "...", "secret": "...", "webhook_port": 8454, "encoding_aes_key": "..." }`
pub async fn wecom_add_tenant(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => match channel_sanitize_name(n) {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing 'name' field" })),
            );
        }
    };

    let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("smartbot");
    if !["smartbot", "app", "kf"].contains(&mode) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid mode: must be smartbot, app, or kf" })),
        );
    }

    let corp_id = match channel_validate_field(
        body.get("corp_id").and_then(|v| v.as_str()).unwrap_or(""), "corp_id",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let secret = match channel_validate_field(
        body.get("secret").and_then(|v| v.as_str()).unwrap_or(""), "secret",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let bot_id = body.get("bot_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if bot_id.len() > CHANNEL_FIELD_MAX_LEN || bot_id.chars().any(|c| c.is_control() && c != ' ') {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid bot_id" })));
    }
    let webhook_port = body.get("webhook_port").and_then(|v| v.as_u64()).unwrap_or(8454);
    if !(1..=65535).contains(&webhook_port) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "webhook_port must be between 1 and 65535" })),
        );
    }
    let encoding_aes_key = body.get("encoding_aes_key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if encoding_aes_key.len() > CHANNEL_FIELD_MAX_LEN || encoding_aes_key.chars().any(|c| c.is_control() && c != ' ') {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid encoding_aes_key" })));
    }

    // Build config as toml::Value
    let mut cfg = toml::value::Table::new();
    cfg.insert("mode".into(), toml::Value::String(mode.to_string()));
    cfg.insert("corp_id".into(), toml::Value::String(corp_id.to_string()));
    if !bot_id.is_empty() {
        cfg.insert("bot_id".into(), toml::Value::String(bot_id.to_string()));
    }
    cfg.insert("secret".into(), toml::Value::String(secret.to_string()));
    cfg.insert("webhook_port".into(), toml::Value::Integer(webhook_port as i64));
    if !encoding_aes_key.is_empty() {
        cfg.insert("encoding_aes_key".into(), toml::Value::String(encoding_aes_key.to_string()));
    }

    let toml_path = state.kernel.config.home_dir
        .join("plugins")
        .join("opencarrier-plugin-wecom")
        .join("plugin.toml");

    if let Err(e) = plugin_toml_add_tenant(&toml_path, &name, toml::Value::Table(cfg)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, mode, "WeCom tenant added via dashboard");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "name": name })))
}
/// POST `/api/channels/feishu/tenants` — add a Feishu tenant to plugin.toml.
///
/// Body: `{ "name": "...", "app_id": "...", "app_secret": "...", "brand": "feishu"|"lark" }`
pub async fn feishu_add_tenant(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => match channel_sanitize_name(n) {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing 'name' field" })),
            );
        }
    };

    let app_id = match channel_validate_field(
        body.get("app_id").and_then(|v| v.as_str()).unwrap_or(""), "app_id",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let app_secret = match channel_validate_field(
        body.get("app_secret").and_then(|v| v.as_str()).unwrap_or(""), "app_secret",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let brand = body.get("brand").and_then(|v| v.as_str()).unwrap_or("feishu");

    if !["feishu", "lark"].contains(&brand) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid brand: must be feishu or lark" })),
        );
    }

    let mut cfg = toml::value::Table::new();
    cfg.insert("app_id".into(), toml::Value::String(app_id.to_string()));
    cfg.insert("app_secret".into(), toml::Value::String(app_secret.to_string()));
    cfg.insert("brand".into(), toml::Value::String(brand.to_string()));

    let toml_path = state.kernel.config.home_dir
        .join("plugins")
        .join("opencarrier-plugin-feishu")
        .join("plugin.toml");

    if let Err(e) = plugin_toml_add_tenant(&toml_path, &name, toml::Value::Table(cfg)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, brand, "Feishu tenant added via dashboard");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "name": name })))
}



// ---------------------------------------------------------------------------
// WeChat helpers
// ---------------------------------------------------------------------------

/// WeChat iLink API base URL.
const WEIXIN_ILINK_BASE: &str = "https://ilinkai.weixin.qq.com";
/// iLink bot_type for personal account.
const WEIXIN_BOT_TYPE: u32 = 3;

/// Validate tenant name: only alphanumeric, hyphen, underscore. Prevents path traversal.
fn weixin_sanitize_tenant(name: &str) -> Option<&str> {
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    if name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        Some(name)
    } else {
        None
    }
}

/// Build a shared reqwest client for iLink API calls (no-redirect, no proxy tricks).
fn weixin_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

/// Validate that a baseurl from iLink response is safe (must match known iLink domain).
fn weixin_validate_baseurl(url: &str) -> bool {
    url.starts_with("https://ilinkai.weixin.qq.com")
        || url.starts_with("https://ilinkai.weixin.qq.com/")
}

/// Atomic file write: write to `<path>.tmp` then rename over target.
fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp_path = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)
}

/// Maximum length for config string fields (corp_id, secret, etc.).
const CHANNEL_FIELD_MAX_LEN: usize = 512;

/// Validate a config string field: non-empty after trim, max length, no control chars.
fn channel_validate_field(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    if trimmed.len() > CHANNEL_FIELD_MAX_LEN {
        return Err(format!("{field_name} exceeds max length ({CHANNEL_FIELD_MAX_LEN} chars)"));
    }
    if trimmed.chars().any(|c| c.is_control() && c != ' ') {
        return Err(format!("{field_name} contains invalid characters"));
    }
    Ok(trimmed.to_string())
}

/// Read-modify-write a plugin.toml, appending a new [[tenants]] entry.
fn plugin_toml_add_tenant(
    toml_path: &std::path::Path,
    tenant_name: &str,
    config: toml::Value,
) -> Result<(), String> {
    let lock_path = {
        let mut s = toml_path.as_os_str().to_owned();
        s.push(".lock");
        std::path::PathBuf::from(s)
    };

    if let Some(parent) = toml_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Failed to create plugin directory".to_string())?;
    }

    let lock_file = std::fs::File::create(&lock_path)
        .map_err(|_| "Failed to create lock file".to_string())?;
    lock_file.lock_exclusive()
        .map_err(|_| "Failed to acquire config lock".to_string())?;

    let result = plugin_toml_add_tenant_locked(toml_path, tenant_name, config);

    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);

    result
}

/// Inner implementation (caller must hold the lock).
fn plugin_toml_add_tenant_locked(
    toml_path: &std::path::Path,
    tenant_name: &str,
    config: toml::Value,
) -> Result<(), String> {
    let mut doc = if toml_path.exists() {
        let content = std::fs::read_to_string(toml_path)
            .map_err(|_| "Failed to read plugin config".to_string())?;
        content
            .parse::<toml::Value>()
            .map_err(|_| "Failed to parse plugin config".to_string())?
    } else {
        toml::Value::Table(Default::default())
    };

    let table = doc.as_table_mut().ok_or("Invalid plugin config structure".to_string())?;
    if !table.contains_key("tenants") {
        table.insert("tenants".into(), toml::Value::Array(Vec::new()));
    }
    let tenants = table
        .get_mut("tenants")
        .and_then(|v| v.as_array_mut())
        .ok_or("Invalid tenants section".to_string())?;

    if tenants.iter().any(|t| {
        t.get("name").and_then(|v| v.as_str()) == Some(tenant_name)
    }) {
        return Err(format!("Tenant '{tenant_name}' already exists"));
    }

    let mut entry = toml::value::Table::new();
    entry.insert("name".into(), toml::Value::String(tenant_name.to_string()));
    entry.insert("config".into(), config);
    tenants.push(toml::Value::Table(entry));

    let content = toml::to_string_pretty(&doc)
        .map_err(|_| "Failed to serialize plugin config".to_string())?;
    atomic_write(toml_path, &content)
        .map_err(|_| "Failed to write plugin config".to_string())?;

    Ok(())
}

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/weixin/qrcode", routing::get(weixin_qrcode))
        .route("/api/weixin/qrcode-status", routing::get(weixin_qrcode_status))
        .route("/api/weixin/status", routing::get(weixin_status))
        .route("/api/channels/status", routing::get(channels_status))
        .route("/api/channels/wecom/tenants", routing::post(wecom_add_tenant))
        .route("/api/channels/feishu/tenants", routing::post(feishu_add_tenant))
}
