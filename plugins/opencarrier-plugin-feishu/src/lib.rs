//! Feishu/Lark Bot plugin for OpenCarrier.
//!
//! Provides a WebSocket-based channel adapter that receives events and
//! replies to messages via the Feishu open platform API.
//!
//! **Flow**: app_id/app_secret → tenant_access_token (auto-refresh) →
//! WebSocket long-connection → im.message.receive_v1 → reply.

// Phase 1 only uses text; WS event types are kept for future phases.
#![allow(dead_code)]

use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::{Arc, LazyLock};
use types::FeishuTenantConfig;

mod api;
mod api_ext;
mod channel;
mod token;
mod tools;
mod types;
mod ws;

// ---------------------------------------------------------------------------
// Global tenant registry
// ---------------------------------------------------------------------------

use dashmap::DashMap;
use token::TenantTokenCache;

/// Runtime entry stored in FEISHU_TENANTS — config + pre-built token cache.
pub(crate) struct FeishuTenantEntry {
    pub config: FeishuTenantConfig,
    pub token_cache: Arc<TenantTokenCache>,
}

impl FeishuTenantEntry {
    pub fn new(config: FeishuTenantConfig) -> Self {
        let api_base = config.api_base().to_string();
        let token_cache = Arc::new(TenantTokenCache::new(
            config.app_id.clone(),
            config.app_secret.clone(),
            &api_base,
        ));
        Self { config, token_cache }
    }
}

/// Global registry of all configured Feishu tenants.
pub(crate) static FEISHU_TENANTS: LazyLock<DashMap<String, FeishuTenantEntry>> =
    LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct FeishuPlugin;

impl Plugin for FeishuPlugin {
    const NAME: &'static str = "feishu";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        // Parse tenant configurations from plugin.toml
        for tenant_config in &config.tenants {
            let name = tenant_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() {
                tracing::warn!("Skipping Feishu tenant with empty name");
                continue;
            }

            let app_id = tenant_config["app_id"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let app_secret = tenant_config["app_secret"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if app_id.is_empty() || app_secret.is_empty() {
                tracing::warn!(
                    tenant = %name,
                    "Skipping Feishu tenant: missing app_id or app_secret"
                );
                continue;
            }

            let brand = tenant_config["brand"]
                .as_str()
                .unwrap_or("feishu")
                .to_string();

            let cfg = FeishuTenantConfig {
                name: name.clone(),
                app_id,
                app_secret,
                brand,
            };

            tracing::info!(
                tenant = %name,
                brand = %cfg.brand,
                "Registered Feishu tenant"
            );

            FEISHU_TENANTS.insert(name, FeishuTenantEntry::new(cfg));
        }

        let tenant_count = FEISHU_TENANTS.len();
        if tenant_count == 0 {
            tracing::info!("Feishu plugin loaded (no tenants configured)");
        } else {
            tracing::info!("Feishu plugin loaded with {tenant_count} tenant(s)");
        }

        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ChannelAdapter>> {
        FEISHU_TENANTS
            .iter()
            .map(|entry| {
                let ch = channel::FeishuChannel::from_entry(entry.value());
                Box::new(ch) as Box<dyn opencarrier_plugin_sdk::ChannelAdapter>
            })
            .collect()
    }

    fn tools(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ToolProvider>> {
        tools::build_all_tools()
    }
}

opencarrier_plugin_sdk::declare_plugin!(FeishuPlugin);
