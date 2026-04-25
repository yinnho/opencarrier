//! WeChat Work (WeCom) plugin for OpenCarrier.
//!
//! Provides:
//! - **Channel**: receives messages via webhook (App/Kf) or WebSocket (SmartBot)
//! - **MCP Tools**: 36+ tools via WeCom MCP protocol (doc/msg/contact/todo/meeting/schedule)
//! - **Channel Tool**: send_wecom_message (direct REST API for App/Kf message delivery)
//!
//! Multi-tenant: configure multiple tenants in plugin.toml `[[tenants]]`.
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
        // Parse tenant configurations
        for tenant_config in &config.tenants {
            let mode = tenant_config["mode"].as_str().unwrap_or("app");
            let name = tenant_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() {
                tracing::warn!("Skipping tenant with empty name");
                continue;
            }

            // Read secret from env var
            let secret_env = tenant_config["secret_env"]
                .as_str()
                .unwrap_or("WECOM_SECRET");
            let secret = std::env::var(secret_env).unwrap_or_default();

            // Read MCP bot credentials (optional, for App/Kf modes)
            let mcp_bot_id = tenant_config["mcp_bot_id"]
                .as_str()
                .map(|s| s.to_string());
            let mcp_bot_secret = tenant_config["mcp_bot_secret_env"]
                .as_str()
                .and_then(|env_name| std::env::var(env_name).ok())
                .or_else(|| {
                    tenant_config["mcp_bot_secret"]
                        .as_str()
                        .map(|s| s.to_string())
                });

            match mode {
                "smartbot" => {
                    let bot_id = tenant_config["bot_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if bot_id.is_empty() {
                        tracing::warn!(tenant = %name, "Skipping smartbot tenant with empty bot_id");
                        continue;
                    }

                    let corp_id_for_bot = tenant_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    let entry = token::TenantEntry::new_smartbot(
                        name.clone(),
                        corp_id_for_bot,
                        bot_id,
                        secret,
                    );
                    tracing::info!(tenant = %name, mode = "smartbot", "Registered WeCom smartbot tenant");
                    TOKEN_MANAGER.add_tenant(entry);
                }
                "kf" => {
                    let corp_id = tenant_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let open_kfid = tenant_config["open_kfid"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if corp_id.is_empty() || open_kfid.is_empty() {
                        tracing::warn!(tenant = %name, "Skipping kf tenant: missing corp_id or open_kfid");
                        continue;
                    }

                    let webhook_port = tenant_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
                    let encoding_aes_key = tenant_config["encoding_aes_key"]
                        .as_str()
                        .map(|s| s.to_string());
                    let callback_token = tenant_config["callback_token"]
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
                        tenant = %name,
                        mode = "kf",
                        port = webhook_port,
                        "Registered WeCom kf tenant"
                    );
                    TOKEN_MANAGER.add_tenant(entry);
                }
                _ => {
                    // "app" mode (default)
                    let corp_id = tenant_config["corp_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let agent_id = tenant_config["agent_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if corp_id.is_empty() {
                        tracing::warn!(tenant = %name, "Skipping app tenant with empty corp_id");
                        continue;
                    }

                    let webhook_port = tenant_config["webhook_port"].as_u64().unwrap_or(8454) as u16;
                    let encoding_aes_key = tenant_config["encoding_aes_key"]
                        .as_str()
                        .map(|s| s.to_string());
                    let callback_token = tenant_config["callback_token"]
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
                        tenant = %name,
                        mode = "app",
                        port = webhook_port,
                        "Registered WeCom app tenant"
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
        let mut tools: Vec<Box<dyn ToolProvider>> = vec![Box::new(tools::SendMessageTool)];
        tools.extend(mcp::build_mcp_tools());
        tracing::info!(tool_count = tools.len(), "Registered WeCom plugin tools");
        tools
    }
}

opencarrier_plugin_sdk::declare_plugin!(WeComPlugin);
