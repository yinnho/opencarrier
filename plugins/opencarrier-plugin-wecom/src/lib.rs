//! WeChat Work (WeCom) plugin for OpenCarrier.
//!
//! Provides:
//! - **Channel**: receives messages via webhook (App/Kf) or WebSocket (SmartBot)
//! - **MCP Tools**: 36+ tools via WeCom MCP protocol (doc/msg/contact/todo/meeting/schedule)
//! - **Channel Tool**: send_wecom_message (direct REST API for App/Kf message delivery)
//!
//! Multi-tenant: bots discovered from `<plugin-dir>/<bot-uuid>/bot.toml`.
//! Three modes: `app` (企业应用), `kf` (微信客服), `smartbot` (智能机器人).

use std::sync::LazyLock;

use opencarrier_plugin_sdk::{
    ChannelAdapter, Plugin, PluginConfig, PluginContext, PluginError, ToolProvider,
};
use token::TokenManager;

mod channel;
mod crypto;
mod mcp;
mod smartbot;
mod token;
mod tools;

// ---------------------------------------------------------------------------
// Global token manager (shared across all tenants)
// ---------------------------------------------------------------------------

static TOKEN_MANAGER: LazyLock<TokenManager> = LazyLock::new(TokenManager::new);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct WeComPlugin;

impl Plugin for WeComPlugin {
    const NAME: &'static str = "wecom";
    const VERSION: &'static str = "1.0.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        // Parse bot configurations (discovered from <plugin>/<uuid>/bot.toml)
        for bot_config in &config.bots {
            let bot_uuid = bot_config["_bot_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let mode = bot_config["mode"].as_str().unwrap_or("app");
            let name = bot_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() || bot_uuid.is_empty() {
                tracing::warn!("Skipping bot with empty name or bot_id");
                continue;
            }

            // Read secret: try env var first, fall back to inline config value
            let secret_env = bot_config["secret_env"]
                .as_str()
                .unwrap_or("WECOM_SECRET");
            let secret = match std::env::var(secret_env) {
                Ok(s) if !s.is_empty() => s,
                _ => {
                    let inline = bot_config["secret"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
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
            let mcp_bot_id = bot_config["mcp_bot_id"]
                .as_str()
                .map(|s| s.to_string());
            let mcp_bot_secret = bot_config["mcp_bot_secret_env"]
                .as_str()
                .and_then(|env_name| std::env::var(env_name).ok())
                .or_else(|| {
                    bot_config["mcp_bot_secret"]
                        .as_str()
                        .map(|s| s.to_string())
                });

            match mode {
                "smartbot" => {
                    let wecom_bot_id = bot_config["bot_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if wecom_bot_id.is_empty() {
                        tracing::warn!(bot = %name, "Skipping smartbot with empty bot_id");
                        continue;
                    }

                    let corp_id_for_bot = bot_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    let entry = token::TenantEntry::new_smartbot(
                        bot_uuid.clone(),
                        corp_id_for_bot,
                        wecom_bot_id,
                        secret,
                    );
                    tracing::info!(
                        bot = %name,
                        bot_uuid = %bot_uuid,
                        mode = "smartbot",
                        "Registered WeCom smartbot"
                    );
                    TOKEN_MANAGER.add_tenant(entry);
                }
                "kf" => {
                    let corp_id = bot_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let open_kfid = bot_config["open_kfid"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if corp_id.is_empty() || open_kfid.is_empty() {
                        tracing::warn!(bot = %name, "Skipping kf bot: missing corp_id or open_kfid");
                        continue;
                    }

                    let webhook_port = bot_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
                    let encoding_aes_key = bot_config["encoding_aes_key"]
                        .as_str()
                        .map(|s| s.to_string());
                    let callback_token = bot_config["callback_token"]
                        .as_str()
                        .map(|s| s.to_string());

                    let entry = token::TenantEntry::new_kf(
                        bot_uuid.clone(),
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
                    TOKEN_MANAGER.add_tenant(entry);
                }
                _ => {
                    // "app" mode (default)
                    let corp_id = bot_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let agent_id = bot_config["agent_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if corp_id.is_empty() {
                        tracing::warn!(bot = %name, "Skipping app bot with empty corp_id");
                        continue;
                    }

                    let webhook_port = bot_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
                    let encoding_aes_key = bot_config["encoding_aes_key"]
                        .as_str()
                        .map(|s| s.to_string());
                    let callback_token = bot_config["callback_token"]
                        .as_str()
                        .map(|s| s.to_string());

                    let entry = token::TenantEntry::new_app(
                        bot_uuid.clone(),
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
                    TOKEN_MANAGER.add_tenant(entry);
                }
            }
        }

        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn ChannelAdapter>> {
        let mut channels: Vec<Box<dyn ChannelAdapter>> = Vec::new();

        for entry in TOKEN_MANAGER.tenants.iter() {
            match &entry.mode {
                token::WecomMode::SmartBot { .. } => {
                    // SmartBot uses WebSocket, no webhook server needed
                    let ch = smartbot::SmartBotChannel::new(
                        entry.name.clone(),
                        entry.corp_id.clone(),
                        entry.bot_id().unwrap().to_string(),
                        entry.bot_secret().unwrap().to_string(),
                    );
                    channels.push(Box::new(ch));
                }
                token::WecomMode::App { .. } | token::WecomMode::Kf { .. } => {
                    // App/Kf use webhook server
                    let is_kf = entry.open_kfid().is_some();
                    let ch = channel::WeComChannel::new(
                        entry.name.clone(),
                        entry.corp_id.clone(),
                        entry.webhook_port,
                        entry.encoding_aes_key.clone(),
                        entry.callback_token.clone(),
                        is_kf,
                    );
                    channels.push(Box::new(ch));
                }
            }
        }

        channels
    }

    fn tools(&self) -> Vec<Box<dyn ToolProvider>> {
        let mut tools: Vec<Box<dyn ToolProvider>> = vec![
            Box::new(tools::SendMessageTool),
            Box::new(tools::BotGenerateTool),
            Box::new(tools::BotPollTool),
            Box::new(tools::QrCodeTool),
            Box::new(tools::BotRegisterTool),
            Box::new(tools::BotBindTool),
        ];
        tools.extend(mcp::build_mcp_tools());
        tracing::info!(tool_count = tools.len(), "Registered WeCom plugin tools");
        tools
    }
}

opencarrier_plugin_sdk::declare_plugin!(WeComPlugin);
