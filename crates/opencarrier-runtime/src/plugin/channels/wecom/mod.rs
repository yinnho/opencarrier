//! WeChat Work (WeCom) built-in channel adapter and tools.
//!
//! Provides:
//! - `WeComAppKfWatcher` — discovers app/kf bots from bot.toml and spawns webhook servers
//! - `WeComSmartBotWatcher` — discovers smartbot bots and spawns WebSocket connections
//! - `SendMessageTool` — send messages via channel REST API
//! - `BotGenerateTool`, `BotPollTool`, `QrCodeTool` — SmartBot creation helpers
//! - `BotRegisterTool`, `BotBindTool` — bot.toml management
//! - 36+ MCP tools via JSON-RPC

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencarrier_types::plugin::PluginMessage;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub mod channel;
pub mod crypto;
pub mod mcp;
pub mod smartbot;
pub mod token;
pub mod tools;

pub use tools::{
    BotBindTool, BotGenerateTool, BotPollTool, BotRegisterTool, QrCodeTool, SendMessageTool,
};

// ---------------------------------------------------------------------------
// Global token manager (shared across all tenants)
// ---------------------------------------------------------------------------

static TOKEN_MANAGER: std::sync::LazyLock<token::TokenManager> =
    std::sync::LazyLock::new(token::TokenManager::new);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_plugin_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("OPENCARRIER_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".opencarrier"))
        })?;

    for dir_name in ["wecom", "opencarrier-plugin-wecom"] {
        let dir = home.join("plugins").join(dir_name);
        if dir.exists() {
            return Some(dir);
        }
    }

    Some(home.join("plugins").join("wecom"))
}

fn scan_bot_configs(plugin_dir: &Path) -> Vec<(String, serde_json::Value)> {
    let mut configs = Vec::new();
    let bot_dir = plugin_dir.join("bot");

    let entries = match std::fs::read_dir(&bot_dir) {
        Ok(e) => e,
        Err(_) => return configs,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let bot_uuid = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let bot_toml = path.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }
        match std::fs::read_to_string(&bot_toml) {
            Ok(content) => {
                if let Ok(mut val) = content.parse::<toml::Value>() {
                    if let Some(table) = val.as_table_mut() {
                        table.insert(
                            "_bot_id".to_string(),
                            toml::Value::String(bot_uuid.clone()),
                        );
                    }
                    if let Ok(json) = serde_json::to_value(val) {
                        configs.push((bot_uuid, json));
                    }
                }
            }
            Err(e) => warn!(path = %bot_toml.display(), "Failed to read bot.toml: {e}"),
        }
    }

    configs
}

fn load_bot_config(bot_config: &serde_json::Value) -> Option<token::TenantEntry> {
    let bot_uuid = bot_config["_bot_id"].as_str().unwrap_or("").to_string();
    let mode = bot_config["mode"].as_str().unwrap_or("app");
    let name = bot_config["name"].as_str().unwrap_or("").to_string();

    if name.is_empty() || bot_uuid.is_empty() {
        tracing::warn!("Skipping bot with empty name or bot_id");
        return None;
    }

    // Read secret: try env var first, fall back to inline config value
    let secret_env = bot_config["secret_env"].as_str().unwrap_or("WECOM_SECRET");
    let secret = match std::env::var(secret_env) {
        Ok(s) if !s.is_empty() => s,
        _ => {
            let inline = bot_config["secret"].as_str().unwrap_or("").to_string();
            if !inline.is_empty() {
                tracing::warn!(
                    bot = %name,
                    env_var = %secret_env,
                    "Using inline secret from config — consider setting env var instead"
                );
            }
            inline
        }
    };

    // Read MCP bot credentials (optional, for App/Kf modes)
    let mcp_bot_id = bot_config["mcp_bot_id"].as_str().map(|s| s.to_string());
    let mcp_bot_secret = bot_config["mcp_bot_secret_env"]
        .as_str()
        .and_then(|env_name| std::env::var(env_name).ok())
        .or_else(|| bot_config["mcp_bot_secret"].as_str().map(|s| s.to_string()));

    match mode {
        "smartbot" => {
            let wecom_bot_id = bot_config["bot_id"].as_str().unwrap_or("").to_string();

            if wecom_bot_id.is_empty() {
                tracing::warn!(bot = %name, "Skipping smartbot with empty bot_id");
                return None;
            }

            let corp_id_for_bot = bot_config["corp_id"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let entry = token::TenantEntry::new_smartbot(
                name.clone(),
                corp_id_for_bot,
                wecom_bot_id,
                secret,
            );
            tracing::info!(bot = %name, bot_uuid = %bot_uuid, mode = "smartbot", "Registered WeCom smartbot");
            Some(entry)
        }
        "kf" => {
            let corp_id = bot_config["corp_id"].as_str().unwrap_or("").to_string();
            let open_kfid = bot_config["open_kfid"].as_str().unwrap_or("").to_string();

            if corp_id.is_empty() || open_kfid.is_empty() {
                tracing::warn!(bot = %name, "Skipping kf bot: missing corp_id or open_kfid");
                return None;
            }

            let webhook_port = bot_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
            let encoding_aes_key = bot_config["encoding_aes_key"]
                .as_str()
                .map(|s| s.to_string());
            let callback_token = bot_config["callback_token"]
                .as_str()
                .map(|s| s.to_string());

            let entry = token::TenantEntry::new_kf(
                name.clone(),
                corp_id,
                open_kfid,
                secret,
                webhook_port,
                encoding_aes_key,
                callback_token,
                mcp_bot_id,
                mcp_bot_secret,
            );

            tracing::info!(
                bot = %name,
                bot_uuid = %bot_uuid,
                mode = "kf",
                port = webhook_port,
                "Registered WeCom kf bot"
            );
            Some(entry)
        }
        _ => {
            // "app" mode (default)
            let corp_id = bot_config["corp_id"].as_str().unwrap_or("").to_string();
            let agent_id = bot_config["agent_id"].as_str().unwrap_or("").to_string();

            if corp_id.is_empty() {
                tracing::warn!(bot = %name, "Skipping app bot with empty corp_id");
                return None;
            }

            let webhook_port = bot_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
            let encoding_aes_key = bot_config["encoding_aes_key"]
                .as_str()
                .map(|s| s.to_string());
            let callback_token = bot_config["callback_token"]
                .as_str()
                .map(|s| s.to_string());

            let entry = token::TenantEntry::new_app(
                name.clone(),
                corp_id,
                agent_id,
                secret,
                webhook_port,
                encoding_aes_key,
                callback_token,
                mcp_bot_id,
                mcp_bot_secret,
            );

            tracing::info!(
                bot = %name,
                bot_uuid = %bot_uuid,
                mode = "app",
                port = webhook_port,
                "Registered WeCom app bot"
            );
            Some(entry)
        }
    }
}

// ---------------------------------------------------------------------------
// WeComAppKfWatcher — watches for app/kf bots and spawns webhook servers
// ---------------------------------------------------------------------------

/// Watcher for WeCom App and Kf mode bots.
///
/// Scans `plugins/wecom/bot/<uuid>/bot.toml` on start, loads tenants into
/// `TOKEN_MANAGER`, and spawns a `WeComChannel` for each app/kf bot.
pub struct WeComAppKfWatcher {
    shutdown: Arc<AtomicBool>,
}

impl WeComAppKfWatcher {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for WeComAppKfWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::plugin::BuiltinChannel for WeComAppKfWatcher {
    fn channel_type(&self) -> &str {
        "wecom"
    }

    fn name(&self) -> &str {
        "WeCom App/Kf Watcher"
    }

    fn tenant_id(&self) -> &str {
        ""
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let plugin_dir = find_plugin_dir()
            .ok_or_else(|| "Cannot find WeCom plugin directory".to_string())?;

        let configs = scan_bot_configs(&plugin_dir);
        if configs.is_empty() {
            info!("No WeCom bot configs found, watcher idle");
            return Ok(());
        }

        for (_uuid, config) in configs {
            let entry = match load_bot_config(&config) {
                Some(e) => e,
                None => continue,
            };

            match &entry.mode {
                token::WecomMode::SmartBot { .. } => {
                    // Skip smartbot — handled by SmartBotWatcher
                    continue;
                }
                token::WecomMode::App { .. } | token::WecomMode::Kf { .. } => {
                    let is_kf = entry.open_kfid().is_some();
                    let tenant_name = entry.name.clone();
                    let corp_id = entry.corp_id.clone();
                    let webhook_port = entry.webhook_port;
                    let encoding_aes_key = entry.encoding_aes_key.clone();
                    let callback_token = entry.callback_token.clone();

                    TOKEN_MANAGER.add_tenant(entry);

                    let tx = sender.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("Failed to create tokio runtime for WeCom channel");
                        rt.block_on(async {
                            let mut ch = channel::WeComChannel::new(
                                tenant_name.clone(),
                                corp_id,
                                webhook_port,
                                encoding_aes_key,
                                callback_token,
                                is_kf,
                            );
                            if let Err(e) = ch.start(tx) {
                                warn!(tenant = %tenant_name, "WeCom channel start error: {e}");
                            }
                        });
                    });
                }
            }
        }

        info!("WeCom App/Kf watcher started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let tenant = TOKEN_MANAGER
            .get_tenant(tenant_id)
            .ok_or_else(|| format!("Unknown tenant: {tenant_id}"))?;

        match &tenant.mode {
            token::WecomMode::App { .. } => {
                token::send_app_message(&tenant, user_id, text)?;
            }
            token::WecomMode::Kf { .. } => {
                token::send_kf_message(&tenant, user_id, text)?;
            }
            token::WecomMode::SmartBot { .. } => {
                return Err(
                    "SmartBot mode does not support send via app/kf watcher".to_string(),
                );
            }
        }

        Ok(())
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// WeComSmartBotWatcher — watches for smartbot bots and spawns WS connections
// ---------------------------------------------------------------------------

/// Watcher for WeCom SmartBot bots.
///
/// Scans `plugins/wecom/bot/<uuid>/bot.toml` on start, loads tenants into
/// `TOKEN_MANAGER`, and spawns a `SmartBotChannel` for each smartbot.
pub struct WeComSmartBotWatcher {
    shutdown: Arc<AtomicBool>,
}

impl WeComSmartBotWatcher {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for WeComSmartBotWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::plugin::BuiltinChannel for WeComSmartBotWatcher {
    fn channel_type(&self) -> &str {
        "wecom_smartbot"
    }

    fn name(&self) -> &str {
        "WeCom SmartBot Watcher"
    }

    fn tenant_id(&self) -> &str {
        ""
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let plugin_dir = find_plugin_dir()
            .ok_or_else(|| "Cannot find WeCom plugin directory".to_string())?;

        let configs = scan_bot_configs(&plugin_dir);
        if configs.is_empty() {
            info!("No WeCom smartbot configs found, watcher idle");
            return Ok(());
        }

        for (_uuid, config) in configs {
            let entry = match load_bot_config(&config) {
                Some(e) => e,
                None => continue,
            };

            match &entry.mode {
                token::WecomMode::SmartBot { .. } => {
                    let tenant_name = entry.name.clone();
                    let corp_id = entry.corp_id.clone();
                    let bot_id = entry.bot_id().unwrap_or("").to_string();
                    let secret = entry.bot_secret().unwrap_or("").to_string();

                    TOKEN_MANAGER.add_tenant(entry);

                    let tx = sender.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("Failed to create tokio runtime for SmartBot");
                        rt.block_on(async {
                            let mut ch = smartbot::SmartBotChannel::new(
                                tenant_name.clone(),
                                corp_id,
                                bot_id,
                                secret,
                            );
                            if let Err(e) = ch.start(tx) {
                                warn!(tenant = %tenant_name, "SmartBot channel start error: {e}");
                            }
                        });
                    });
                }
                _ => {
                    // Skip app/kf — handled by AppKfWatcher
                    continue;
                }
            }
        }

        info!("WeCom SmartBot watcher started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let key = format!("{}:{}", tenant_id, user_id);
        let response_url = smartbot::RESPONSE_URLS
            .get()
            .ok_or_else(|| "RESPONSE_URLS not initialized".to_string())?
            .lock()
            .unwrap()
            .remove(&key)
            .ok_or_else(|| {
                "No response_url available for this user. SmartBot can only reply within callback context."
                    .to_string()
            })?;

        let tenant = TOKEN_MANAGER
            .get_tenant(tenant_id)
            .ok_or_else(|| format!("Unknown tenant: {tenant_id}"))?;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Runtime creation failed: {e}"))?;
        rt.block_on(token::send_smartbot_response_async(
            &tenant.http, &response_url, text,
        ))
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}
