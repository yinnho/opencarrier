//! WeChat iLink Bot, WeCom, and Feishu channel endpoints.

use crate::routes::plugin_toml::*;
use crate::routes::state::AppState;
use axum::extract::{Path, Query, State};
use opencarrier_kernel::KernelHandle;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;
/// GET `/api/weixin/qrcode` — fetch a fresh QR code for WeChat scanning.
///
/// Query params: `?tenant=<name>` (optional, defaults to "default")
pub async fn weixin_qrcode(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let raw_tenant = params
        .get("tenant")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let tenant = match weixin_sanitize_tenant(raw_tenant) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" }),
                ),
            );
        }
    };

    let url = format!("{WEIXIN_ILINK_BASE}/ilink/bot/get_bot_qrcode?bot_type={WEIXIN_BOT_TYPE}");

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
        Ok(data) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "tenant": tenant,
                "data": data,
            })),
        ),
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
    let raw_tenant = params
        .get("tenant")
        .map(|s| s.as_str())
        .unwrap_or("default");
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
        let bot_token = data.get("bot_token").and_then(|v| v.as_str()).unwrap_or("");
        let raw_baseurl = data
            .get("baseurl")
            .and_then(|v| v.as_str())
            .unwrap_or(WEIXIN_ILINK_BASE);
        let ilink_bot_id = data
            .get("ilink_bot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ilink_user_id = data.get("ilink_user_id").and_then(|v| v.as_str());

        // Check if this WeChat user already has a tenant (dedup by user_id)
        let token_dir = state.kernel.config.home_dir.join("weixin-tokens");
        if let Some(uid) = ilink_user_id {
            if let Ok(entries) = std::fs::read_dir(&token_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                            let existing_uid = tf.get("user_id").and_then(|v| v.as_str());
                            if existing_uid == Some(uid) {
                                let existing_tenant = tf
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(tenant)
                                    .to_string();
                                let existing_bind = tf.get("bind_agent").and_then(|v| v.as_str()).map(|s| s.to_string());

                                tracing::info!(
                                    new_tenant = %tenant,
                                    existing_tenant = %existing_tenant,
                                    user_id = %uid,
                                    "WeChat user already has a tenant, reusing"
                                );

                                // Update the existing token file with new bot_token/expires
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64;
                                let mut updated = tf.clone();
                                updated["bot_token"] = serde_json::Value::String(bot_token.to_string());
                                updated["baseurl"] = serde_json::Value::String(raw_baseurl.to_string());
                                updated["ilink_bot_id"] =
                                    serde_json::Value::String(ilink_bot_id.to_string());
                                updated["expires_at"] = serde_json::Value::Number(
                                    serde_json::Number::from(now + 86400),
                                );

                                // Look up real tenant_id (UUID) from tenant store
                                let real_tenant_id = state.kernel.memory.tenant()
                                    .get_tenant_by_name(&existing_tenant)
                                    .ok()
                                    .flatten()
                                    .map(|t| t.id)
                                    .unwrap_or_else(|| existing_tenant.clone());

                                // Ensure token file has tenant_id
                                updated["tenant_id"] = serde_json::Value::String(real_tenant_id.clone());

                                if let Ok(json) = serde_json::to_string_pretty(&updated) {
                                    let _ = atomic_write(&path, &json);
                                }

                                // Register dynamic bridge binding if bind_agent exists
                                if let Some(ref agent_id) = existing_bind {
                                    if !agent_id.is_empty() && uuid::Uuid::parse_str(agent_id).is_ok() {
                                        if let Some(ref pm_arc) = state.plugin_manager {
                                            let pm = pm_arc.lock().await;
                                            pm.add_channel_binding("weixin", &existing_tenant, agent_id);
                                            state.kernel.set_default_plugin_tenant(agent_id, &existing_tenant);
                                            tracing::info!(
                                                tenant = %existing_tenant,
                                                agent = %agent_id,
                                                "Dynamically bound WeChat bot to agent (rebind)"
                                            );
                                        }
                                    }
                                }

                                // Create session token with real tenant_id
                                let api_key = state.kernel.config.api_key.trim().to_string();
                                let secret = if !api_key.is_empty() {
                                    api_key
                                } else {
                                    state.kernel.config.auth.password_hash.clone()
                                };
                                let session_token = crate::session_auth::create_session_token(
                                    Some(&real_tenant_id), "tenant", &existing_tenant, &secret, 24,
                                );

                                return (
                                    StatusCode::OK,
                                    Json(serde_json::json!({
                                        "tenant": existing_tenant,
                                        "tenant_id": real_tenant_id,
                                        "status": "confirmed",
                                        "existing": true,
                                        "bind_agent": existing_bind,
                                        "session_token": session_token,
                                        "data": data,
                                    })),
                                );
                            }
                        }
                    }
                }
            }
        }

        // New user — don't save token here, return iLink data for frontend to save after onboard
        let baseurl = if weixin_validate_baseurl(raw_baseurl) {
            raw_baseurl
        } else {
            WEIXIN_ILINK_BASE
        };

        tracing::info!(
            tenant,
            ilink_bot_id,
            user_id = ?ilink_user_id,
            "WeChat iLink QR scan confirmed — new user, returning ilink data"
        );

        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "tenant": tenant,
                "status": "confirmed",
                "existing": false,
                "ilink_bot_id": ilink_bot_id,
                "ilink_user_id": ilink_user_id,
                "bot_token": bot_token,
                "baseurl": baseurl,
                "data": data,
            })),
        );
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

/// POST `/api/weixin/save-token` — save ilink token for a new user after onboard.
pub async fn weixin_save_token(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let tenant_name = match body.get("tenant_name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing tenant_name"})),
            );
        }
    };
    let tenant_id = body.get("tenant_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let bot_token = body.get("bot_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ilink_bot_id = body.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ilink_user_id = body.get("ilink_user_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    let baseurl = body.get("baseurl").and_then(|v| v.as_str()).unwrap_or(WEIXIN_ILINK_BASE).to_string();

    if bot_token.is_empty() || ilink_bot_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing bot_token or ilink_bot_id"})),
        );
    }

    let token_dir = state.kernel.config.home_dir.join("weixin-tokens");
    if let Err(e) = std::fs::create_dir_all(&token_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create token dir: {e}")})),
        );
    }

    let bind_agent = body.get("bind_agent").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let token_data = serde_json::json!({
        "name": tenant_name,
        "tenant_id": tenant_id,
        "bot_token": bot_token,
        "baseurl": baseurl,
        "ilink_bot_id": ilink_bot_id,
        "user_id": ilink_user_id,
        "expires_at": now + 86400,
        "bind_agent": if bind_agent.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(bind_agent.clone()) },
    });

    // Use ilink_user_id as filename for dedup; fallback to tenant_name
    let filename = ilink_user_id.as_deref().unwrap_or(&tenant_name);
    let path = token_dir.join(format!("{filename}.json"));
    match serde_json::to_string_pretty(&token_data) {
        Ok(json) => {
            if let Err(e) = atomic_write(&path, &json) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to save token: {e}")})),
                );
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Serialization error: {e}")})),
            );
        }
    }

    // Register dynamic bridge binding if bind_agent is provided
    if !bind_agent.is_empty() && uuid::Uuid::parse_str(&bind_agent).is_ok() {
        if let Some(ref pm_arc) = state.plugin_manager {
            let pm = pm_arc.lock().await;
            pm.add_channel_binding("weixin", &tenant_name, &bind_agent);
            state.kernel.set_default_plugin_tenant(&bind_agent, &tenant_name);
            tracing::info!(
                tenant = %tenant_name,
                agent = %bind_agent,
                "Dynamically bound WeChat bot to agent"
            );
        }
    }

    tracing::info!(tenant = %tenant_name, ilink_bot_id, "WeChat ilink token saved");
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true})),
    )
}

/// GET `/api/weixin/status` — list all bound WeChat accounts with expiry info.
pub async fn weixin_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
                        let expires_at = tf.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
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
pub async fn channels_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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

    // ── WeCom & Feishu — scan all plugin dirs for bot.toml ───────
    let plugins_dir = home.join("plugins");
    let mut wecom_tenants: Vec<serde_json::Value> = Vec::new();
    let mut feishu_tenants: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }
            let toml_path = plugin_dir.join("plugin.toml");
            if !toml_path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&toml_path) else {
                continue;
            };
            let Ok(doc) = content.parse::<toml::Value>() else {
                continue;
            };

            // Determine channel category from [[channels]]
            let has_wecom = doc
                .get("channels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|ch| {
                        ch.get("channel_type")
                            .and_then(|v| v.as_str())
                            .map(|t| t.starts_with("wecom"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);

            let has_feishu = doc
                .get("channels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|ch| {
                        ch.get("channel_type")
                            .and_then(|v| v.as_str())
                            .map(|t| t == "feishu" || t == "lark")
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);

            if !has_wecom && !has_feishu {
                continue;
            }

            // Scan bot/<uuid>/bot.toml files
            let bot_root = plugin_dir.join("bot");
            if let Ok(sub_entries) = std::fs::read_dir(&bot_root) {
                for sub_entry in sub_entries.flatten() {
                    let bot_dir = sub_entry.path();
                    if !bot_dir.is_dir() {
                        continue;
                    }
                    let bot_toml = bot_dir.join("bot.toml");
                    if !bot_toml.exists() {
                        continue;
                    }

                    let Ok(bt) = std::fs::read_to_string(&bot_toml) else {
                        continue;
                    };
                    let Ok(bt_doc) = bt.parse::<toml::Value>() else {
                        continue;
                    };

                    let name = bt_doc
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let bind_agent = bt_doc
                        .get("bind_agent")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mode = bt_doc
                        .get("mode")
                        .and_then(|v| v.as_str())
                        .unwrap_or("smartbot");
                    let bot_uuid = bot_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    if has_wecom {
                        let corp_id = bt_doc.get("corp_id").and_then(|v| v.as_str()).unwrap_or("");
                        let bot_id = bt_doc.get("bot_id").and_then(|v| v.as_str()).unwrap_or("");
                        let secret_env = bt_doc
                            .get("secret_env")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        wecom_tenants.push(serde_json::json!({
                            "name": name,
                            "bot_uuid": bot_uuid,
                            "mode": mode,
                            "corp_id": corp_id,
                            "bot_id": bot_id,
                            "secret_env": secret_env,
                            "bind_agent": bind_agent,
                        }));
                    }
                    if has_feishu {
                        let app_id = bt_doc.get("app_id").and_then(|v| v.as_str()).unwrap_or("");
                        let app_secret_env = bt_doc
                            .get("app_secret_env")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let brand = bt_doc
                            .get("brand")
                            .and_then(|v| v.as_str())
                            .unwrap_or("feishu");
                        feishu_tenants.push(serde_json::json!({
                            "name": name,
                            "bot_uuid": bot_uuid,
                            "app_id": app_id,
                            "app_secret_env": app_secret_env,
                            "brand": brand,
                            "bind_agent": bind_agent,
                        }));
                    }
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

/// POST `/api/channels/wecom/tenants` — add a WeCom bot (creates bot.toml).
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
                    Json(
                        serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" }),
                    ),
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

    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("smartbot");
    if !["smartbot", "app", "kf"].contains(&mode) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid mode: must be smartbot, app, or kf" })),
        );
    }

    let corp_id = match channel_validate_field(
        body.get("corp_id").and_then(|v| v.as_str()).unwrap_or(""),
        "corp_id",
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };
    let secret = match channel_validate_field(
        body.get("secret").and_then(|v| v.as_str()).unwrap_or(""),
        "secret",
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };
    let bot_id = body
        .get("bot_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if bot_id.len() > CHANNEL_FIELD_MAX_LEN || bot_id.chars().any(|c| c.is_control() && c != ' ') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid bot_id" })),
        );
    }
    let webhook_port = body
        .get("webhook_port")
        .and_then(|v| v.as_u64())
        .unwrap_or(8454);
    if !(1..=65535).contains(&webhook_port) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "webhook_port must be between 1 and 65535" })),
        );
    }
    let encoding_aes_key = body
        .get("encoding_aes_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if encoding_aes_key.len() > CHANNEL_FIELD_MAX_LEN
        || encoding_aes_key.chars().any(|c| c.is_control() && c != ' ')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid encoding_aes_key" })),
        );
    }

    // Build bot.toml fields
    let mut cfg = toml::value::Table::new();
    cfg.insert("name".into(), toml::Value::String(name.to_string()));
    cfg.insert("mode".into(), toml::Value::String(mode.to_string()));
    cfg.insert("corp_id".into(), toml::Value::String(corp_id.to_string()));
    if !bot_id.is_empty() {
        cfg.insert("bot_id".into(), toml::Value::String(bot_id.to_string()));
    }
    cfg.insert("secret".into(), toml::Value::String(secret.to_string()));
    cfg.insert(
        "webhook_port".into(),
        toml::Value::Integer(webhook_port as i64),
    );
    if !encoding_aes_key.is_empty() {
        cfg.insert(
            "encoding_aes_key".into(),
            toml::Value::String(encoding_aes_key.to_string()),
        );
    }

    let plugin_dir = state
        .kernel
        .config
        .home_dir
        .join("plugins")
        .join("opencarrier-plugin-wecom");

    if let Err(e) = create_bot_toml(&plugin_dir, &name, cfg) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, mode, "WeCom tenant added via dashboard");
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "name": name })),
    )
}
/// POST `/api/channels/feishu/tenants` — add a Feishu bot (creates bot.toml).
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
                    Json(
                        serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" }),
                    ),
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
        body.get("app_id").and_then(|v| v.as_str()).unwrap_or(""),
        "app_id",
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };
    let app_secret = match channel_validate_field(
        body.get("app_secret")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "app_secret",
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };
    let brand = body
        .get("brand")
        .and_then(|v| v.as_str())
        .unwrap_or("feishu");

    if !["feishu", "lark"].contains(&brand) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid brand: must be feishu or lark" })),
        );
    }

    let mut cfg = toml::value::Table::new();
    cfg.insert("name".into(), toml::Value::String(name.to_string()));
    cfg.insert("app_id".into(), toml::Value::String(app_id.to_string()));
    cfg.insert(
        "app_secret".into(),
        toml::Value::String(app_secret.to_string()),
    );
    cfg.insert("brand".into(), toml::Value::String(brand.to_string()));

    let plugin_dir = state
        .kernel
        .config
        .home_dir
        .join("plugins")
        .join("opencarrier-plugin-feishu");

    if let Err(e) = create_bot_toml(&plugin_dir, &name, cfg) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, brand, "Feishu tenant added via dashboard");
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "name": name })),
    )
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
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
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

/// Create a new bot.toml file in <plugin_dir>/bot/<uuid>/bot.toml.
fn create_bot_toml(
    plugin_dir: &std::path::Path,
    tenant_name: &str,
    fields: toml::value::Table,
) -> Result<(), String> {
    let bot_root = plugin_dir.join("bot");

    // Check duplicate name
    if let Ok(entries) = std::fs::read_dir(&bot_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let bot_toml = path.join("bot.toml");
            if !bot_toml.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&bot_toml) {
                if let Ok(doc) = content.parse::<toml::Value>() {
                    if doc.get("name").and_then(|v| v.as_str()) == Some(tenant_name) {
                        return Err(format!("Bot '{tenant_name}' already exists"));
                    }
                }
            }
        }
    }

    let bot_uuid = uuid::Uuid::new_v4().to_string();
    let bot_dir = bot_root.join(&bot_uuid);
    std::fs::create_dir_all(&bot_dir).map_err(|e| format!("Failed to create bot dir: {e}"))?;

    let content = toml::to_string_pretty(&toml::Value::Table(fields))
        .map_err(|e| format!("Serialize error: {e}"))?;

    atomic_write(&bot_dir.join("bot.toml"), &content).map_err(|e| format!("Write error: {e}"))?;

    tracing::info!(tenant = %tenant_name, bot_uuid = %bot_uuid, "Created bot.toml");
    Ok(())
}

/// POST `/api/weixin/{name}/bind` — bind a WeChat bot to an agent.
pub async fn weixin_bind_bot(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_input = match body.get("agent_name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "缺少 agent_name 字段"})),
            );
        }
    };

    // Resolve agent_name to UUID
    let agent_uuid = if uuid::Uuid::parse_str(&agent_input).is_ok() {
        agent_input.clone()
    } else {
        let agents = state.kernel.list_agents("");
        match agents.iter().find(|a| a.name == agent_input) {
            Some(agent) => agent.id.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("分身 '{agent_input}' 不存在")})),
                );
            }
        }
    };

    let token_dir = state.kernel.config.home_dir.join("weixin-tokens");
    let entries = match std::fs::read_dir(&token_dir) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "微信机器人不存在"})),
            );
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut tf) = serde_json::from_str::<serde_json::Value>(&content) {
                let token_name = tf.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if token_name != name {
                    continue;
                }

                tf["bind_agent"] = serde_json::Value::String(agent_uuid.clone());
                if let Ok(json) = serde_json::to_string_pretty(&tf) {
                    let _ = atomic_write(&path, &json);
                }

                // Register dynamic binding
                if let Some(ref pm_arc) = state.plugin_manager {
                    let pm = pm_arc.lock().await;
                    pm.add_channel_binding("weixin", &name, &agent_uuid);
                    state.kernel.set_default_plugin_tenant(&agent_uuid, &name);
                }

                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "bound",
                        "message": "微信机器人已绑定",
                        "bind_agent": agent_uuid,
                    })),
                );
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "微信机器人不存在"})),
    )
}

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/weixin/qrcode", routing::get(weixin_qrcode))
        .route(
            "/api/weixin/qrcode-status",
            routing::get(weixin_qrcode_status),
        )
        .route("/api/weixin/save-token", routing::post(weixin_save_token))
        .route("/api/weixin/status", routing::get(weixin_status))
        .route("/api/weixin/{name}/bind", routing::post(weixin_bind_bot))
        .route("/api/channels/status", routing::get(channels_status))
        .route(
            "/api/channels/wecom/tenants",
            routing::post(wecom_add_tenant),
        )
        .route(
            "/api/channels/feishu/tenants",
            routing::post(feishu_add_tenant),
        )
}
