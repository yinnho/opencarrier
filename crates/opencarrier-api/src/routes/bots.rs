//! Bot management API — unified management for WeCom/Feishu/DingTalk bots.
//!
//! Bots are stored as `<plugin>/bot/<bot-uuid>/bot.toml` files.
//! This module provides a bot-centric view and CRUD operations.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_kernel::KernelHandle;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::routes::plugin_toml::*;
use crate::routes::state::AppState;

/// Detect platform from plugin's [[channels]] declarations.
fn detect_platform(doc: &toml::Value) -> Vec<&str> {
    let mut platforms = Vec::new();
    let Some(arr) = doc.get("channels").and_then(|v| v.as_array()) else {
        return platforms;
    };
    for ch in arr {
        if let Some(ct) = ch.get("channel_type").and_then(|v| v.as_str()) {
            if ct.starts_with("wecom") && !platforms.contains(&"wecom") {
                platforms.push("wecom");
            }
            if (ct == "feishu" || ct == "lark") && !platforms.contains(&"feishu") {
                platforms.push("feishu");
            }
            if ct.starts_with("dingtalk") && !platforms.contains(&"dingtalk") {
                platforms.push("dingtalk");
            }
            if ct == "weixin" && !platforms.contains(&"weixin") {
                platforms.push("weixin");
            }
        }
    }
    platforms
}

/// Map plugin directory name to platform identifier for known plugins.
fn plugin_dir_to_platform(dir_name: &str) -> Option<&str> {
    if dir_name.contains("wecom") {
        Some("wecom")
    } else if dir_name.contains("feishu") {
        Some("feishu")
    } else if dir_name.contains("dingtalk") {
        Some("dingtalk")
    } else if dir_name.contains("weixin") {
        Some("weixin")
    } else {
        None
    }
}

/// Plugin directory names for each platform.
fn platform_plugin_dir(platform: &str) -> Option<&str> {
    match platform {
        "wecom" | "wecom_smartbot" => Some("wecom"),
        "feishu" => Some("feishu"),
        "weixin" => Some("weixin"),
        "dingtalk" => Some("dingtalk"),
        _ => None,
    }
}

/// Scan a plugin directory for bot.toml files in `bot/<uuid>/bot.toml`.
fn scan_bots(
    plugin_dir: &std::path::Path,
    dir_name: &str,
    platform: &str,
) -> Vec<serde_json::Value> {
    let mut bots = Vec::new();
    let bot_root = plugin_dir.join("bot");
    let entries = match std::fs::read_dir(&bot_root) {
        Ok(e) => e,
        Err(_) => return bots,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let bot_toml = path.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }
        let bot_uuid = match path.file_name().and_then(|n| n.to_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let Ok(content) = std::fs::read_to_string(&bot_toml) else {
            continue;
        };
        let Ok(doc) = content.parse::<toml::Value>() else {
            continue;
        };

        let tenant_name = doc
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let bind_agent = doc.get("bind_agent").and_then(|v| v.as_str()).unwrap_or("");

        let mut bot = serde_json::json!({
            "id": bot_uuid,
            "platform": platform,
            "plugin": dir_name,
            "tenant_name": tenant_name,
            "mode": doc.get("mode").and_then(|v| v.as_str()).unwrap_or(""),
            "bind_agent": if bind_agent.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(bind_agent.to_string()) },
            "owner_id": doc.get("owner_id").and_then(|v| v.as_str()).map(|s| serde_json::Value::String(s.to_string())).unwrap_or(serde_json::Value::Null),
            "status": "configured",
        });

        let obj = bot.as_object_mut().unwrap();
        match platform {
            "wecom" => {
                obj.insert(
                    "bot_id".into(),
                    serde_json::Value::String(
                        doc.get("bot_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
                obj.insert(
                    "corp_id".into(),
                    serde_json::Value::String(
                        doc.get("corp_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
            }
            "feishu" => {
                obj.insert(
                    "app_id".into(),
                    serde_json::Value::String(
                        doc.get("app_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
                obj.insert(
                    "brand".into(),
                    serde_json::Value::String(
                        doc.get("brand")
                            .and_then(|v| v.as_str())
                            .unwrap_or("feishu")
                            .to_string(),
                    ),
                );
            }
            "dingtalk" => {
                obj.insert(
                    "app_key".into(),
                    serde_json::Value::String(
                        doc.get("app_key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
            }
            "weixin" => {
                obj.insert(
                    "ilink_user_id".into(),
                    serde_json::Value::String(
                        doc.get("ilink_user_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
                obj.insert(
                    "ilink_bot_id".into(),
                    serde_json::Value::String(
                        doc.get("ilink_bot_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
                obj.insert(
                    "bot_token".into(),
                    serde_json::Value::String(
                        doc.get("bot_token")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                );
            }
            _ => {}
        }

        bots.push(bot);
    }

    bots
}

// ---------------------------------------------------------------------------
// GET /api/bots — list all bots
// ---------------------------------------------------------------------------

pub async fn list_bots(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");
    let mut bots: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }
            let dir_name = plugin_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let toml_path = plugin_dir.join("plugin.toml");
            let platform = if toml_path.exists() {
                let Ok(content) = std::fs::read_to_string(&toml_path) else {
                    continue;
                };
                let Ok(doc) = content.parse::<toml::Value>() else {
                    continue;
                };
                detect_platform(&doc)
                    .first()
                    .copied()
                    .or_else(|| plugin_dir_to_platform(dir_name))
                    .map(|p| p.to_string())
            } else {
                plugin_dir_to_platform(dir_name).map(|p| p.to_string())
            };
            let Some(ref platform) = platform else { continue };

            bots.extend(scan_bots(&plugin_dir, dir_name, platform));
        }
    }

    // ── WeChat iLink bots from token files (fallback for bots without bot.toml) ──
    let token_dir = home.join("weixin-tokens");
    if token_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&token_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                        let name = tf.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let ilink_bot_id = tf.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or("");
                        let bind_agent = tf.get("bind_agent").and_then(|v| v.as_str());
                        let user_id = tf.get("user_id").and_then(|v| v.as_str()).unwrap_or("");

                        // Deduplicate: skip if this tenant already has a bot.toml entry
                        let already_exists = bots.iter().any(|b| {
                            b.get("tenant_name").and_then(|v| v.as_str()) == Some(name)
                        });
                        if already_exists {
                            continue;
                        }

                        bots.push(serde_json::json!({
                            "id": user_id,
                            "platform": "weixin",
                            "plugin": "weixin",
                            "tenant_name": name,
                            "mode": "ilink",
                            "bind_agent": bind_agent,
                            "owner_id": null,
                            "status": "configured",
                            "ilink_bot_id": ilink_bot_id,
                        }));
                    }
                }
            }
        }
    }

    Json(serde_json::json!({
        "bots": bots,
        "count": bots.len(),
    }))
}

// ---------------------------------------------------------------------------
// POST /api/bots/wecom/smartbot/generate — step 1: get auth URL
// ---------------------------------------------------------------------------

pub async fn wecom_smartbot_generate() -> impl IntoResponse {
    let http = reqwest::Client::new();
    let url = "https://work.weixin.qq.com/ai/qc/generate?source=wecom_cli_external&plat=1";

    match http.get(url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(data) => {
                    let inner = data.get("data").unwrap_or(&data);
                    let scode = inner.get("scode").and_then(|v| v.as_str()).unwrap_or("");
                    if scode.is_empty() {
                        return (
                            StatusCode::BAD_GATEWAY,
                            Json(serde_json::json!({ "error": "WeCom API 返回了空的 scode" })),
                        );
                    }
                    let auth_url = inner
                        .get("auth_url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            format!(
                                "https://work.weixin.qq.com/ai/qc/gen?source=wecom_cli_external&scode={scode}"
                            )
                        });
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "scode": scode,
                            "auth_url": auth_url,
                        })),
                    )
                }
                Err(_) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "无法解析 WeCom API 响应" })),
                ),
            },
            Err(_) => (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "无法读取 WeCom API 响应" })),
            ),
        },
        Err(_) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "无法连接 WeCom API" })),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /api/bots/wecom/smartbot/poll — step 2: poll creation result
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PollQuery {
    scode: String,
}

pub async fn wecom_smartbot_poll(Query(query): Query<PollQuery>) -> impl IntoResponse {
    if query.scode.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing scode parameter" })),
        );
    }

    let http = reqwest::Client::new();
    let url = format!(
        "https://work.weixin.qq.com/ai/qc/query_result?scode={}",
        query.scode
    );

    match http.get(&url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(data) => {
                    let inner = data.get("data").unwrap_or(&data);
                    let status = inner
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let mut result = serde_json::json!({
                        "status": status,
                    });

                    if status == "success" {
                        if let Some(bot_info) = inner.get("bot_info") {
                            let bot_id =
                                bot_info.get("botid").and_then(|v| v.as_str()).unwrap_or("");
                            let secret = bot_info
                                .get("secret")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            result.as_object_mut().unwrap().insert(
                                "bot_id".into(),
                                serde_json::Value::String(bot_id.to_string()),
                            );
                            result.as_object_mut().unwrap().insert(
                                "secret".into(),
                                serde_json::Value::String(secret.to_string()),
                            );
                        }
                    }

                    (StatusCode::OK, Json(result))
                }
                Err(_) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "无法解析 WeCom API 响应" })),
                ),
            },
            Err(_) => (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "无法读取 WeCom API 响应" })),
            ),
        },
        Err(_) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "无法连接 WeCom API" })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Shared in-memory store for device-auth flows
// ---------------------------------------------------------------------------

#[derive(Clone)]
#[allow(dead_code)]
struct DeviceAuthSession {
    device_code: String,
    auth_url: String,
    expires_at: std::time::Instant,
    platform: String,
    // stored credentials after poll success
    credentials: Option<serde_json::Value>,
}

static DEVICE_AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, DeviceAuthSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn cleanup_expired_sessions() {
    let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
    let now = std::time::Instant::now();
    sessions.retain(|_, v| v.expires_at > now);
}

// ---------------------------------------------------------------------------
// Feishu device-auth: POST /api/bots/feishu/device-auth
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FeishuDeviceAuthBeginBody {
    #[serde(default)]
    brand: String,
}

pub async fn feishu_device_auth_begin(
    Json(body): Json<FeishuDeviceAuthBeginBody>,
) -> impl IntoResponse {
    cleanup_expired_sessions();

    let brand = if body.brand.trim().is_empty() {
        "feishu"
    } else {
        &body.brand
    };

    let base_url = if brand == "lark" {
        "https://accounts.larksuite.com"
    } else {
        "https://accounts.feishu.cn"
    };

    let http = reqwest::Client::new();

    // Step 1: init
    let init_url = format!("{}/oauth/v1/app/registration", base_url);
    let init_res = match http
        .post(&init_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("action=init")
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("飞书 init 响应解析失败: {}", e) })),
                )
            }
        },
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("飞书 init 请求失败: {}", e) })),
            )
        }
    };

    let nonce = init_res
        .get("nonce")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if nonce.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": "飞书 init 未返回 nonce" })),
        );
    }

    // Step 2: begin
    let begin_body = format!(
        "action=begin&nonce={}&archetype=PersonalAgent&auth_method=client_secret&request_user_info=open_id",
        nonce
    );
    let begin_res = match http
        .post(&init_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(begin_body)
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("飞书 begin 响应解析失败: {}", e) })),
                )
            }
        },
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("飞书 begin 请求失败: {}", e) })),
            )
        }
    };

    let device_code = begin_res
        .get("device_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let auth_url = begin_res
        .get("verification_uri_complete")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if device_code.is_empty() || auth_url.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": "飞书 begin 未返回 device_code 或 auth_url", "raw": begin_res })),
        );
    }

    let session_id = generate_session_id();
    let expires_in = begin_res
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(240u64);

    let session = DeviceAuthSession {
        device_code: device_code.to_string(),
        auth_url: auth_url.to_string(),
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(expires_in),
        platform: "feishu".to_string(),
        credentials: None,
    };

    {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.insert(session_id.clone(), session);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "device_code": device_code,
            "auth_url": auth_url,
            "expires_in": expires_in,
        })),
    )
}

#[derive(Deserialize)]
pub struct FeishuDeviceAuthPollQuery {
    session_id: String,
}

pub async fn feishu_device_auth_poll(
    Query(query): Query<FeishuDeviceAuthPollQuery>,
) -> impl IntoResponse {
    cleanup_expired_sessions();

    let session = {
        let sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        match sessions.get(&query.session_id) {
            Some(s) => s.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "会话不存在或已过期" })),
                )
            }
        }
    };

    if let Some(creds) = session.credentials {
        return (StatusCode::OK, Json(creds));
    }

    if std::time::Instant::now() > session.expires_at {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.remove(&query.session_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired" })),
        );
    }

    let base_url = if session.platform == "lark" {
        "https://accounts.larksuite.com"
    } else {
        "https://accounts.feishu.cn"
    };

    let http = reqwest::Client::new();
    let poll_url = format!("{}/oauth/v1/app/registration", base_url);
    let poll_body = format!("action=poll&device_code={}", session.device_code);

    let poll_res = match http
        .post(&poll_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(poll_body)
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "pending" })),
                )
            }
        },
        Err(_) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending" })),
            )
        }
    };

    let status = poll_res
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if status == "SUCCESS" {
        let app_id = poll_res
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let app_secret = poll_res
            .get("app_secret")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !app_id.is_empty() && !app_secret.is_empty() {
            let result = serde_json::json!({
                "status": "success",
                "app_id": app_id,
                "app_secret": app_secret,
            });

            let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
            if let Some(s) = sessions.get_mut(&query.session_id) {
                s.credentials = Some(result.clone());
            }

            return (StatusCode::OK, Json(result));
        }
    }

    if status == "EXPIRED" || status == "FAIL" {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.remove(&query.session_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired" })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "pending" })),
    )
}

// ---------------------------------------------------------------------------
// DingTalk device-auth: POST /api/bots/dingtalk/device-auth
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct DingtalkDeviceAuthPollQuery {
    session_id: String,
}

pub async fn dingtalk_device_auth_begin() -> impl IntoResponse {
    cleanup_expired_sessions();

    let http = reqwest::Client::new();
    let base_url = "https://oapi.dingtalk.com";

    // Step 1: init
    let init_res = match http
        .post(format!("{}/app/registration/init", base_url))
        .json(&serde_json::json!({"source": "opencarrier"}))
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("钉钉 init 响应解析失败: {}", e) })),
                )
            }
        },
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("钉钉 init 请求失败: {}", e) })),
            )
        }
    };

    let nonce = init_res
        .get("nonce")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if nonce.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": "钉钉 init 未返回 nonce" })),
        );
    }

    // Step 2: begin
    let begin_res = match http
        .post(format!("{}/app/registration/begin", base_url))
        .json(&serde_json::json!({"nonce": nonce}))
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("钉钉 begin 响应解析失败: {}", e) })),
                )
            }
        },
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("钉钉 begin 请求失败: {}", e) })),
            )
        }
    };

    let device_code = begin_res
        .get("device_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let auth_url = begin_res
        .get("verification_uri_complete")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if device_code.is_empty() || auth_url.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": "钉钉 begin 未返回 device_code 或 auth_url", "raw": begin_res })),
        );
    }

    let session_id = generate_session_id();
    let expires_in = begin_res
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(7200u64);

    let session = DeviceAuthSession {
        device_code: device_code.to_string(),
        auth_url: auth_url.to_string(),
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(expires_in),
        platform: "dingtalk".to_string(),
        credentials: None,
    };

    {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.insert(session_id.clone(), session);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "device_code": device_code,
            "auth_url": auth_url,
            "expires_in": expires_in,
        })),
    )
}

pub async fn dingtalk_device_auth_poll(
    Query(query): Query<DingtalkDeviceAuthPollQuery>,
) -> impl IntoResponse {
    cleanup_expired_sessions();

    let session = {
        let sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        match sessions.get(&query.session_id) {
            Some(s) => s.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "会话不存在或已过期" })),
                )
            }
        }
    };

    if let Some(creds) = session.credentials {
        return (StatusCode::OK, Json(creds));
    }

    if std::time::Instant::now() > session.expires_at {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.remove(&query.session_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired" })),
        );
    }

    let http = reqwest::Client::new();
    let poll_res = match http
        .post("https://oapi.dingtalk.com/app/registration/poll")
        .json(&serde_json::json!({"device_code": session.device_code}))
        .send()
        .await
    {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "pending" })),
                )
            }
        },
        Err(_) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending" })),
            )
        }
    };

    let status = poll_res
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();

    if status == "SUCCESS" {
        let client_id = poll_res
            .get("client_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let client_secret = poll_res
            .get("client_secret")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !client_id.is_empty() && !client_secret.is_empty() {
            let result = serde_json::json!({
                "status": "success",
                "client_id": client_id,
                "client_secret": client_secret,
            });

            let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
            if let Some(s) = sessions.get_mut(&query.session_id) {
                s.credentials = Some(result.clone());
            }

            return (StatusCode::OK, Json(result));
        }
    }

    if status == "EXPIRED" || status == "FAIL" {
        let mut sessions = DEVICE_AUTH_SESSIONS.lock().unwrap();
        sessions.remove(&query.session_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired" })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "pending" })),
    )
}

// ---------------------------------------------------------------------------
// POST /api/bots — create a bot (write to <uuid>/bot.toml)
// ---------------------------------------------------------------------------

/// Extract the platform-specific user id used for deduplication.
fn platform_user_id(platform: &str, body: &serde_json::Value) -> Option<String> {
    match platform {
        "weixin" => body
            .get("ilink_user_id")
            .or_else(|| body.get("bot_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "wecom" => body
            .get("bot_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "feishu" => body
            .get("app_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "dingtalk" => body
            .get("client_id")
            .or_else(|| body.get("app_key"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Build bot.toml fields from request body for a given platform.
fn build_bot_fields(platform: &str, body: &serde_json::Value, tenant_name: &str) -> toml::value::Table {
    let mut bot_fields = toml::value::Table::new();
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("smartbot");
    bot_fields.insert("mode".into(), toml::Value::String(mode.to_string()));

    match platform {
        "wecom" => {
            if let Some(v) = body.get("corp_id").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("corp_id".into(), toml::Value::String(v.to_string()));
                }
            }
            if let Some(v) = body.get("bot_id").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("bot_id".into(), toml::Value::String(v.to_string()));
                }
            }
            let secret_env = format!("WECOM_BOT_SECRET_{tenant_name}").to_uppercase();
            bot_fields.insert("secret_env".into(), toml::Value::String(secret_env));
            if let Some(v) = body.get("secret").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("secret".into(), toml::Value::String(v.to_string()));
                }
            }
        }
        "feishu" => {
            if let Some(v) = body.get("app_id").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert(
                        "app_id".into(),
                        toml::Value::String(
                            channel_validate_field(v, "app_id").unwrap_or_default(),
                        ),
                    );
                }
            }
            if let Some(v) = body.get("app_secret").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("app_secret".into(), toml::Value::String(v.to_string()));
                }
            }
            let brand = body
                .get("brand")
                .and_then(|v| v.as_str())
                .unwrap_or("feishu");
            bot_fields.insert("brand".into(), toml::Value::String(brand.to_string()));
        }
        "dingtalk" => {
            if let Some(v) = body.get("app_key").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("app_key".into(), toml::Value::String(v.to_string()));
                }
            }
            if let Some(v) = body.get("app_secret").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("app_secret".into(), toml::Value::String(v.to_string()));
                }
            }
            if let Some(v) = body.get("corp_id").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("corp_id".into(), toml::Value::String(v.to_string()));
                }
            }
        }
        "weixin" => {
            if let Some(v) = body.get("ilink_user_id").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("ilink_user_id".into(), toml::Value::String(v.to_string()));
                }
            }
            if let Some(v) = body.get("bot_token").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    bot_fields.insert("bot_token".into(), toml::Value::String(v.to_string()));
                }
            }
        }
        _ => {}
    }
    bot_fields
}

pub async fn create_bot(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let tenant_name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => match channel_sanitize_name(n) {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({ "error": "名称无效：仅支持字母、数字、连字符、下划线（最多64字符）" }),
                    ),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "缺少 name 字段" })),
            );
        }
    };

    let platform = body.get("platform").and_then(|v| v.as_str()).unwrap_or("");
    let plugin_dir_name = match platform_plugin_dir(platform) {
        Some(d) => d,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "不支持的平台，支持: wecom, feishu, dingtalk, weixin" })),
            );
        }
    };

    let home = &state.kernel.config.home_dir;
    let plugin_dir = home.join("plugins").join(plugin_dir_name);

    if !plugin_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&plugin_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("创建插件目录失败: {e}") })),
            );
        }
    }

    let puid = platform_user_id(platform, &body);

    // ── Deduplication: if same (platform, platform_user_id) exists, update it ──
    if let Some(ref uid) = puid {
        if !uid.is_empty() {
        for existing in scan_bots(&plugin_dir, plugin_dir_name, platform) {
            let existing_puid = match platform {
                "weixin" => existing
                    .get("ilink_user_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| existing.get("ilink_bot_id").and_then(|v| v.as_str())),
                "wecom" => existing.get("bot_id").and_then(|v| v.as_str()),
                "feishu" => existing.get("app_id").and_then(|v| v.as_str()),
                "dingtalk" => existing
                    .get("app_key")
                    .and_then(|v| v.as_str()),
                _ => None,
            };
            if existing_puid == Some(uid.as_str()) {
                let existing_id = existing
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let bot_dir = plugin_dir.join("bot").join(&existing_id);
                let bot_toml_path = bot_dir.join("bot.toml");

                // Merge new fields into existing bot.toml
                return match update_bot_toml(&bot_toml_path, |table| {
                    let new_fields = build_bot_fields(platform, &body, &tenant_name);
                    for (k, v) in new_fields {
                        table.insert(k, v);
                    }
                    table.insert("name".into(), toml::Value::String(tenant_name.to_string()));
                }) {
                    Ok(()) => (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "updated",
                            "message": "机器人已更新",
                            "bot_id": existing_id,
                        })),
                    ),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e })),
                    ),
                };
            }
        }
        }
    }

    // Build bot.toml fields
    let mut bot_fields = build_bot_fields(platform, &body, &tenant_name);

    // bind_agent — resolve agent name to UUID
    if let Some(v) = body.get("bind_agent").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            let agent_uuid = if uuid::Uuid::parse_str(v).is_ok() {
                v.to_string()
            } else {
                let agents = state.kernel.registry.list();
                match agents.iter().find(|a| a.name == v) {
                    Some(agent) => agent.id.to_string(),
                    None => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({ "error": format!("分身 '{v}' 不存在") })),
                        );
                    }
                }
            };
            bot_fields.insert("bind_agent".into(), toml::Value::String(agent_uuid));
        }
    }

    // Generate UUID for the bot
    let bot_uuid = uuid::Uuid::new_v4().to_string();
    let bot_dir = plugin_dir.join("bot").join(&bot_uuid);

    // Create bot directory and write bot.toml
    if let Err(e) = std::fs::create_dir_all(&bot_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("创建目录失败: {e}") })),
        );
    }

    bot_fields.insert("name".into(), toml::Value::String(tenant_name.clone()));
    let bot_toml_path = bot_dir.join("bot.toml");
    let content = match toml::to_string_pretty(&toml::Value::Table(bot_fields)) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("序列化失败: {e}") })),
            );
        }
    };

    if let Err(e) = std::fs::write(&bot_toml_path, &content) {
        let _ = std::fs::remove_dir_all(&bot_dir);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("写入失败: {e}") })),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "created",
            "message": "机器人已创建",
            "bot_id": bot_uuid,
        })),
    )
}

// ---------------------------------------------------------------------------
// DELETE /api/bots/{bot_uuid} — delete a bot
// ---------------------------------------------------------------------------

pub async fn delete_bot(
    State(state): State<Arc<AppState>>,
    Path(bot_uuid): Path<String>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");

    // Find the bot directory across all plugins
    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "插件目录不存在" })),
            );
        }
    };

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let bot_dir = plugin_dir.join("bot").join(&bot_uuid);
        let bot_toml = bot_dir.join("bot.toml");
        if bot_toml.exists() {
            match std::fs::remove_dir_all(&bot_dir) {
                Ok(()) => {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "deleted",
                            "message": "机器人已删除，重启后生效",
                        })),
                    );
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("删除失败: {e}") })),
                    );
                }
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "机器人不存在" })),
    )
}

// ---------------------------------------------------------------------------
// PUT /api/bots/{bot_uuid}/bind — bind bot to agent
// ---------------------------------------------------------------------------

pub async fn bind_bot(
    State(state): State<Arc<AppState>>,
    Path(bot_uuid): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_input = match body.get("agent_name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "缺少 agent_name 字段" })),
            );
        }
    };

    // Resolve agent_name to UUID — bind_agent must be a UUID, not a name
    let agent_uuid = if uuid::Uuid::parse_str(&agent_input).is_ok() {
        agent_input.clone()
    } else {
        let agents = state.kernel.list_agents("");
        match agents.iter().find(|a| a.name == agent_input) {
            Some(agent) => agent.id.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": format!("分身 '{agent_input}' 不存在") })),
                );
            }
        }
    };

    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");

    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "插件目录不存在" })),
            );
        }
    };

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let bot_dir = plugin_dir.join("bot").join(&bot_uuid);
        let bot_toml = bot_dir.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }

        return match update_bot_toml(&bot_toml, |table| {
            table.insert("bind_agent".into(), toml::Value::String(agent_uuid.clone()));
        }) {
            Ok(()) => {
                // Add dynamic bridge binding so messages route immediately
                let dir_name = plugin_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                let platform = plugin_dir_to_platform(dir_name);
                if let Some(platform) = platform {
                    let tenant_name = std::fs::read_to_string(&bot_toml)
                        .ok()
                        .and_then(|c| c.parse::<toml::Value>().ok())
                        .and_then(|d| {
                            d.get("name")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default();

                    let channel_type = match platform {
                        "weixin" => "weixin",
                        "wecom" => "wecom",
                        "feishu" => "feishu",
                        "dingtalk" => "dingtalk",
                        _ => "",
                    };

                    if !channel_type.is_empty() && !tenant_name.is_empty() {
                        // Set default plugin tenant
                        state
                            .kernel
                            .set_default_plugin_tenant(&agent_uuid, &bot_uuid);
                        // Add dynamic bridge bindings
                        if let Some(ref pm) = state.plugin_manager {
                            let pm = pm.lock().await;
                            pm.add_channel_binding(channel_type, &bot_uuid, &agent_uuid);
                            pm.add_channel_binding(channel_type, &tenant_name, &agent_uuid);
                            pm.map_channel_tenant(channel_type, &tenant_name, &bot_uuid);
                            tracing::info!(
                                platform = %platform,
                                tenant = %tenant_name,
                                agent = %agent_uuid,
                                "Dynamic bridge binding added"
                            );
                        }
                    }
                }

                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "bound",
                        "message": "分身已绑定",
                        "bind_agent": agent_uuid,
                    })),
                )
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            ),
        };
    }

    // Fallback: check weixin-tokens for iLink bots
    {
        let token_dir = home.join("weixin-tokens");
        if token_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&token_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let mut tf: serde_json::Value = match serde_json::from_str(&content) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let user_id = tf.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                    if user_id != bot_uuid {
                        continue;
                    }
                    // Found the iLink bot — update bind_agent
                    tf["bind_agent"] = serde_json::Value::String(agent_uuid.clone());
                    if let Ok(updated) = serde_json::to_string_pretty(&tf) {
                        let _ = std::fs::write(&path, updated);
                    }
                    // Add dynamic bridge binding
                    let tenant_name = tf.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if !tenant_name.is_empty() {
                        state.kernel.set_default_plugin_tenant(&agent_uuid, &bot_uuid);
                        if let Some(ref pm) = state.plugin_manager {
                            let pm = pm.lock().await;
                            pm.add_channel_binding("weixin", &bot_uuid, &agent_uuid);
                            pm.add_channel_binding("weixin", tenant_name, &agent_uuid);
                            pm.map_channel_tenant("weixin", tenant_name, &bot_uuid);
                            tracing::info!(
                                platform = "weixin",
                                tenant = %tenant_name,
                                agent = %agent_uuid,
                                "Dynamic bridge binding added (iLink)"
                            );
                        }
                    }
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "bound",
                            "message": "分身已绑定",
                            "bind_agent": agent_uuid,
                        })),
                    );
                }
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "机器人不存在" })),
    )
}

// ---------------------------------------------------------------------------
// DELETE /api/bots/{bot_uuid}/bind — unbind bot from agent
// ---------------------------------------------------------------------------

pub async fn unbind_bot(
    State(state): State<Arc<AppState>>,
    Path(bot_uuid): Path<String>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");

    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "插件目录不存在" })),
            );
        }
    };

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let bot_dir = plugin_dir.join("bot").join(&bot_uuid);
        let bot_toml = bot_dir.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }

        return match update_bot_toml(&bot_toml, |table| {
            table.remove("bind_agent");
        }) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "unbound",
                    "message": "分身已解绑，重启后生效",
                })),
            ),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            ),
        };
    }

    // Fallback: check weixin-tokens for iLink bots
    let token_dir = home.join("weixin-tokens");
    if token_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&token_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let mut tf: serde_json::Value = match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let user_id = tf.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                if user_id != bot_uuid {
                    continue;
                }
                tf.as_object_mut().map(|o| o.remove("bind_agent"));
                if let Ok(updated) = serde_json::to_string_pretty(&tf) {
                    let _ = std::fs::write(&path, updated);
                }
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "unbound",
                        "message": "分身已解绑",
                    })),
                );
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "机器人不存在" })),
    )
}

/// Helper to atomically update a bot.toml file.
fn update_bot_toml(
    path: &std::path::Path,
    f: impl FnOnce(&mut toml::value::Table),
) -> Result<(), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let mut doc = content
        .parse::<toml::Value>()
        .map_err(|e| format!("解析失败: {e}"))?;

    let table = doc
        .as_table_mut()
        .ok_or("Invalid bot.toml structure".to_string())?;

    f(table);

    let new_content = toml::to_string_pretty(&doc).map_err(|e| format!("序列化失败: {e}"))?;
    atomic_write(path, &new_content).map_err(|e| format!("写入失败: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GET /api/bots/{bot_uuid} — get single bot details
// ---------------------------------------------------------------------------

pub async fn get_bot(
    State(state): State<Arc<AppState>>,
    Path(bot_uuid): Path<String>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");

    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }
            let dir_name = plugin_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
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

            let platforms = detect_platform(&doc);
            let platform = platforms
                .first()
                .copied()
                .or_else(|| plugin_dir_to_platform(dir_name));
            let Some(platform) = platform else { continue };

            let bots = scan_bots(&plugin_dir, dir_name, platform);
            if let Some(bot) = bots.into_iter().find(|b| {
                b.get("id").and_then(|v| v.as_str()) == Some(&bot_uuid)
            }) {
                return (StatusCode::OK, Json(bot));
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "机器人不存在" })),
    )
}

// ---------------------------------------------------------------------------
// PUT /api/bots/{bot_uuid} — update bot config
// ---------------------------------------------------------------------------

pub async fn update_bot(
    State(state): State<Arc<AppState>>,
    Path(bot_uuid): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;
    let plugins_dir = home.join("plugins");

    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "插件目录不存在" })),
            );
        }
    };

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let bot_dir = plugin_dir.join("bot").join(&bot_uuid);
        let bot_toml = bot_dir.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }

        return match update_bot_toml(&bot_toml, |table| {
            if let Some(v) = body.get("name").and_then(|v| v.as_str()) {
                table.insert("name".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("mode").and_then(|v| v.as_str()) {
                table.insert("mode".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("corp_id").and_then(|v| v.as_str()) {
                table.insert("corp_id".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("bot_id").and_then(|v| v.as_str()) {
                table.insert("bot_id".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("secret").and_then(|v| v.as_str()) {
                table.insert("secret".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("app_id").and_then(|v| v.as_str()) {
                table.insert("app_id".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("app_secret").and_then(|v| v.as_str()) {
                table.insert("app_secret".into(), toml::Value::String(v.to_string()));
            }
            if let Some(v) = body.get("brand").and_then(|v| v.as_str()) {
                table.insert("brand".into(), toml::Value::String(v.to_string()));
            }
        }) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "updated",
                    "message": "机器人已更新",
                })),
            ),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            ),
        };
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "机器人不存在" })),
    )
}

pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/bots", routing::get(list_bots).post(create_bot))
        .route(
            "/api/bots/wecom/smartbot/generate",
            routing::post(wecom_smartbot_generate),
        )
        .route(
            "/api/bots/wecom/smartbot/poll",
            routing::get(wecom_smartbot_poll),
        )
        .route(
            "/api/bots/feishu/device-auth",
            routing::post(feishu_device_auth_begin),
        )
        .route(
            "/api/bots/feishu/device-auth/poll",
            routing::get(feishu_device_auth_poll),
        )
        .route(
            "/api/bots/dingtalk/device-auth",
            routing::post(dingtalk_device_auth_begin),
        )
        .route(
            "/api/bots/dingtalk/device-auth/poll",
            routing::get(dingtalk_device_auth_poll),
        )
        .route("/api/bots/{bot_uuid}", routing::get(get_bot).put(update_bot).delete(delete_bot))
        .route(
            "/api/bots/{bot_uuid}/bind",
            routing::put(bind_bot).delete(unbind_bot),
        )
}
