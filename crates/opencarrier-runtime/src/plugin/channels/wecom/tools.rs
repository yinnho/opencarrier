//! WeCom tool providers — messaging via channel REST API, bot creation.
//!
//! Document/spreadsheet/contact/todo/meeting/schedule tools are now provided
//! by the MCP module (`crate::plugin::channels::wecom::mcp`).

use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

use crate::plugin::channels::wecom::token;

// ---------------------------------------------------------------------------
// Send WeCom Message (App and Kf modes — uses channel-layer REST API)
// ---------------------------------------------------------------------------

pub struct SendMessageTool;

impl ToolProvider for SendMessageTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "send_wecom_message",
            "发送企业微信消息给指定用户（支持企业应用和微信客服模式）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": { "type": "string", "description": "接收人UserID（企业应用为内部UserID，微信客服为external_userid）" },
                    "content": { "type": "string", "description": "消息内容" }
                },
                "required": ["user_id", "content"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let user_id = args["user_id"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing user_id"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing content"))?;

        let tenant = crate::plugin::channels::wecom::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        match &tenant.mode {
            token::WecomMode::App { .. } => {
                token::send_app_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::Kf { .. } => {
                token::send_kf_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::SmartBot { .. } => {
                return Err(PluginError::tool(
                    "SmartBot mode replies via response_url, not send_wecom_message",
                ));
            }
        }

        Ok(format!("Message sent to {}", user_id))
    }
}

// ---------------------------------------------------------------------------
// Bot Creation Tools (SmartBot)
// ---------------------------------------------------------------------------

pub struct BotGenerateTool;

impl ToolProvider for BotGenerateTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "wecom_bot_generate",
            "生成企业微信智能机器人创建链接（返回 scode 和 auth_url，需要用户扫码完成创建）",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    fn execute(&self, _args: &Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        rt.block_on(async {
            let http = reqwest::Client::new();
            let url = "https://work.weixin.qq.com/ai/qc/generate?source=wecom_cli_external&plat=1";

            let resp = http.get(url).send().await
                .map_err(|e| PluginError::tool(format!("WeCom API request failed: {e}")))?;

            let data: Value = resp.json().await
                .map_err(|e| PluginError::tool(format!("WeCom API parse error: {e}")))?;

            let inner = data.get("data").unwrap_or(&data);
            let scode = inner.get("scode").and_then(|v| v.as_str()).unwrap_or("");

            if scode.is_empty() {
                return Err(PluginError::tool("WeCom API 返回了空的 scode"));
            }

            let auth_url = inner.get("auth_url").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!(
                    "https://work.weixin.qq.com/ai/qc/gen?source=wecom_cli_external&scode={scode}"
                ));

            Ok(serde_json::json!({
                "scode": scode,
                "auth_url": auth_url,
            }).to_string())
        })
    }
}

pub struct BotPollTool;

impl ToolProvider for BotPollTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "wecom_bot_poll",
            "轮询企业微信智能机器人创建结果（传入 scode，返回创建状态和 bot_id/secret）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scode": { "type": "string", "description": "wecom_bot_generate 返回的 scode" }
                },
                "required": ["scode"]
            }),
        )
    }

    fn execute(&self, args: &Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
        let scode = args["scode"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing scode"))?;

        if scode.is_empty() {
            return Err(PluginError::tool("scode cannot be empty"));
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        rt.block_on(async {
            let http = reqwest::Client::new();
            let url = format!("https://work.weixin.qq.com/ai/qc/query_result?scode={scode}");

            let resp = http.get(&url).send().await
                .map_err(|e| PluginError::tool(format!("WeCom API request failed: {e}")))?;

            let data: Value = resp.json().await
                .map_err(|e| PluginError::tool(format!("WeCom API parse error: {e}")))?;

            let inner = data.get("data").unwrap_or(&data);
            let status = inner.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

            let mut result = serde_json::json!({ "status": status });

            if status == "success" {
                if let Some(bot_info) = inner.get("bot_info") {
                    result["bot_id"] = bot_info.get("botid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into();
                    result["secret"] = bot_info.get("secret")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into();
                }
            }

            Ok(result.to_string())
        })
    }
}

// ---------------------------------------------------------------------------
// QR Code Generation
// ---------------------------------------------------------------------------

pub struct QrCodeTool;

impl ToolProvider for QrCodeTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "wecom_bot_qrcode",
            "将链接生成二维码图片（返回 base64 PNG 图片数据，可直接展示给用户）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "需要生成二维码的链接" }
                },
                "required": ["url"]
            }),
        )
    }

    fn execute(&self, args: &Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing url"))?;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        rt.block_on(async {
            let qr_api = format!(
                "https://api.qrserver.com/v1/create-qr-code/?size=300x300&format=png&data={}",
                urlencoding::encode(url)
            );

            let http = reqwest::Client::new();
            let resp = http.get(&qr_api).send().await
                .map_err(|e| PluginError::tool(format!("QR API request failed: {e}")))?;

            if !resp.status().is_success() {
                return Err(PluginError::tool(format!("QR API returned {}", resp.status())));
            }

            let bytes = resp.bytes().await
                .map_err(|e| PluginError::tool(format!("QR API read error: {e}")))?;

            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

            Ok(serde_json::json!({
                "image_base64": b64,
                "image_url": qr_api,
            }).to_string())
        })
    }
}

// ---------------------------------------------------------------------------
// Bot Registration & Binding (write to bot.toml files)
// ---------------------------------------------------------------------------

fn find_plugin_dir() -> Result<std::path::PathBuf, PluginError> {
    let home = std::env::var("OPENCARRIER_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            dirs_home().map(|h| h.join(".opencarrier"))
        })
        .ok_or_else(|| PluginError::tool("Cannot determine OpenCarrier home directory"))?;

    // Try new built-in path first, then legacy path
    for dir_name in ["wecom", "opencarrier-plugin-wecom"] {
        let dir = home.join("plugins").join(dir_name);
        if dir.exists() {
            return Ok(dir);
        }
    }

    // Default to wecom even if not existing yet
    Ok(home.join("plugins").join("wecom"))
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}

fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Find an existing bot directory by searching for a bot.toml with matching name.
fn find_bot_dir_by_name(plugin_dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let bot_dir = plugin_dir.join("bot");
    let entries = std::fs::read_dir(&bot_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let bot_toml = path.join("bot.toml");
        if let Ok(content) = std::fs::read_to_string(&bot_toml) {
            if let Ok(doc) = content.parse::<toml::Value>() {
                if doc.get("name").and_then(|v| v.as_str()) == Some(name) {
                    return Some(path);
                }
            }
        }
    }
    None
}

pub struct BotRegisterTool;

impl ToolProvider for BotRegisterTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "wecom_bot_register",
            "注册企微机器人到系统（创建 bot.toml），需要名称、bot_id、secret，可选 corp_id 和 bind_agent（bind_agent 必须是分身 UUID）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "机器人名称" },
                    "bot_id": { "type": "string", "description": "企微 bot_id" },
                    "secret": { "type": "string", "description": "企微 secret" },
                    "corp_id": { "type": "string", "description": "企业 ID（可选）" },
                    "bind_agent": { "type": "string", "description": "绑定的分身 UUID（可选）" }
                },
                "required": ["name", "bot_id", "secret"]
            }),
        )
    }

    fn execute(&self, args: &Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
        let name = args["name"].as_str().ok_or_else(|| PluginError::tool("Missing name"))?;
        let bot_id = args["bot_id"].as_str().ok_or_else(|| PluginError::tool("Missing bot_id"))?;
        let secret = args["secret"].as_str().ok_or_else(|| PluginError::tool("Missing secret"))?;

        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 64 || !trimmed.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c.is_ascii_punctuation()) {
            return Err(PluginError::tool("Invalid name: use only alphanumeric, hyphen, underscore (max 64 chars)"));
        }

        let plugin_dir = find_plugin_dir()?;

        // Check for duplicate name
        if find_bot_dir_by_name(&plugin_dir, trimmed).is_some() {
            return Err(PluginError::tool(format!("Bot '{trimmed}' already exists")));
        }

        // Generate a new UUID for the bot
        let bot_uuid = uuid::Uuid::new_v4().to_string();
        let bot_dir = plugin_dir.join("bot").join(&bot_uuid);
        std::fs::create_dir_all(&bot_dir)
            .map_err(|e| PluginError::tool(format!("Failed to create bot dir: {e}")))?;

        // Build bot.toml content
        let mut table = toml::value::Table::new();
        table.insert("name".into(), toml::Value::String(trimmed.to_string()));
        table.insert("mode".into(), toml::Value::String("smartbot".into()));
        table.insert("bot_id".into(), toml::Value::String(bot_id.to_string()));
        table.insert("secret".into(), toml::Value::String(secret.to_string()));

        if let Some(corp_id) = args["corp_id"].as_str() {
            if !corp_id.is_empty() {
                table.insert("corp_id".into(), toml::Value::String(corp_id.to_string()));
            }
        }

        if let Some(agent) = args["bind_agent"].as_str() {
            if !agent.is_empty() {
                if uuid::Uuid::parse_str(agent).is_err() {
                    return Err(PluginError::tool("bind_agent must be a valid UUID, not an agent name"));
                }
                table.insert("bind_agent".into(), toml::Value::String(agent.to_string()));
            }
        }

        let content = toml::to_string_pretty(&toml::Value::Table(table))
            .map_err(|e| PluginError::tool(format!("Serialize error: {e}")))?;
        let bot_toml_path = bot_dir.join("bot.toml");
        atomic_write(&bot_toml_path, &content)
            .map_err(|e| PluginError::tool(format!("Write error: {e}")))?;

        tracing::info!(bot = %trimmed, bot_uuid = %bot_uuid, "WeCom SmartBot registered via plugin tool");
        Ok(serde_json::json!({
            "status": "registered",
            "name": trimmed,
            "bot_uuid": bot_uuid,
            "message": "机器人已注册，重启后生效"
        }).to_string())
    }
}

pub struct BotBindTool;

impl ToolProvider for BotBindTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "wecom_bot_bind",
            "将企微机器人绑定到指定分身（bind_agent 必须是分身 UUID）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "机器人名称" },
                    "agent_id": { "type": "string", "description": "分身 UUID" }
                },
                "required": ["name", "agent_id"]
            }),
        )
    }

    fn execute(&self, args: &Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
        let name = args["name"].as_str().ok_or_else(|| PluginError::tool("Missing name"))?;
        let agent_id = args["agent_id"].as_str().ok_or_else(|| PluginError::tool("Missing agent_id"))?;

        if uuid::Uuid::parse_str(agent_id).is_err() {
            return Err(PluginError::tool("agent_id must be a valid UUID, not an agent name"));
        }

        let plugin_dir = find_plugin_dir()?;
        let bot_dir = find_bot_dir_by_name(&plugin_dir, name)
            .ok_or_else(|| PluginError::tool(format!("Bot '{name}' not found")))?;

        let bot_toml_path = bot_dir.join("bot.toml");
        let content = std::fs::read_to_string(&bot_toml_path)
            .map_err(|e| PluginError::tool(format!("Read error: {e}")))?;
        let mut doc = content.parse::<toml::Value>()
            .map_err(|e| PluginError::tool(format!("Parse error: {e}")))?;

        if let Some(table) = doc.as_table_mut() {
            table.insert("bind_agent".into(), toml::Value::String(agent_id.to_string()));
        }

        let new_content = toml::to_string_pretty(&doc)
            .map_err(|e| PluginError::tool(format!("Serialize error: {e}")))?;
        atomic_write(&bot_toml_path, &new_content)
            .map_err(|e| PluginError::tool(format!("Write error: {e}")))?;

        tracing::info!(bot = %name, agent_id = %agent_id, "WeCom bot bound via plugin tool");
        Ok(serde_json::json!({
            "status": "bound",
            "name": name,
            "bind_agent": agent_id,
            "message": "已绑定，重启后生效"
        }).to_string())
    }
}
