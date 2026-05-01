//! Bot management API — unified management for WeCom/Feishu/DingTalk bots.
//!
//! Bots are stored as `<plugin>/<bot-uuid>/bot.toml` files.
//! This module provides a bot-centric view and CRUD operations.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_kernel::KernelHandle;
use serde::Deserialize;
use std::sync::Arc;

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
    } else {
        None
    }
}

/// Plugin directory names for each platform.
fn platform_plugin_dir(platform: &str) -> Option<&str> {
    match platform {
        "wecom" => Some("opencarrier-plugin-wecom"),
        "feishu" => Some("opencarrier-plugin-feishu"),
        "dingtalk" => Some("opencarrier-plugin-dingtalk"),
        _ => None,
    }
}

/// Scan a plugin directory for bot.toml files in subdirectories.
fn scan_bots(
    plugin_dir: &std::path::Path,
    dir_name: &str,
    platform: &str,
) -> Vec<serde_json::Value> {
    let mut bots = Vec::new();
    let entries = match std::fs::read_dir(plugin_dir) {
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

            bots.extend(scan_bots(&plugin_dir, dir_name, platform));
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
// POST /api/bots — create a bot (write to <uuid>/bot.toml)
// ---------------------------------------------------------------------------

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
                Json(serde_json::json!({ "error": "不支持的平台，支持: wecom, feishu, dingtalk" })),
            );
        }
    };

    let home = &state.kernel.config.home_dir;
    let plugin_dir = home.join("plugins").join(plugin_dir_name);

    if !plugin_dir.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "插件目录不存在" })),
        );
    }

    // Build bot.toml fields
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
        _ => {}
    }

    // bind_agent — resolve agent name to UUID
    if let Some(v) = body.get("bind_agent").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            let agent_uuid = if uuid::Uuid::parse_str(v).is_ok() {
                v.to_string()
            } else {
                let agents = state.kernel.list_agents("");
                match agents.iter().find(|a| a.name == v) {
                    Some(agent) => agent.id.clone(),
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
    let bot_dir = plugin_dir.join(&bot_uuid);

    // Check duplicate name
    for existing in scan_bots(&plugin_dir, plugin_dir_name, platform) {
        if existing.get("tenant_name").and_then(|v| v.as_str()) == Some(tenant_name.as_str()) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": format!("机器人 '{tenant_name}' 已存在") })),
            );
        }
    }

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

    // Copy dylib if not already present
    let has_dylib = std::fs::read_dir(&plugin_dir)
        .map(|mut entries| {
            entries.any(|e| {
                e.ok()
                    .and_then(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|s| s == "dylib" || s == "so" || s == "dll")
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if !has_dylib {
        let dylib_name = format!("libopencarrier_plugin_{platform}");
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        let candidates = [
            std::env::var("OPENCARRIER_BUILD_DIR")
                .ok()
                .map(|d| std::path::PathBuf::from(d).join(format!("{dylib_name}.{ext}"))),
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join(format!("{dylib_name}.{ext}")))),
            std::env::current_exe().ok().and_then(|exe| {
                exe.parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("lib").join(format!("{dylib_name}.{ext}")))
            }),
        ];
        let mut copied = false;
        for candidate in candidates.iter().flatten() {
            if candidate.exists() {
                let dest = plugin_dir.join(candidate.file_name().unwrap());
                if let Err(e) = std::fs::copy(candidate, &dest) {
                    tracing::warn!("Failed to copy dylib: {e}");
                } else {
                    copied = true;
                }
                break;
            }
        }
        if !copied {
            tracing::warn!(
                "No dylib found for {dylib_name} — copy it manually to {}",
                plugin_dir.display()
            );
        }
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "created",
            "message": "机器人已创建，重启后生效",
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
        let bot_dir = plugin_dir.join(&bot_uuid);
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
        let bot_dir = plugin_dir.join(&bot_uuid);
        let bot_toml = bot_dir.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }

        return match update_bot_toml(&bot_toml, |table| {
            table.insert("bind_agent".into(), toml::Value::String(agent_uuid.clone()));
        }) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "bound",
                    "message": "分身已绑定，重启后生效",
                    "bind_agent": agent_uuid,
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
        let bot_dir = plugin_dir.join(&bot_uuid);
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
        .route("/api/bots/{bot_uuid}", routing::delete(delete_bot))
        .route(
            "/api/bots/{bot_uuid}/bind",
            routing::put(bind_bot).delete(unbind_bot),
        )
}
