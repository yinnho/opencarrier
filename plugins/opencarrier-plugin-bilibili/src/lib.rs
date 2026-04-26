//! Bilibili platform plugin for OpenCarrier.
//!
//! Provides 13 tools via Bilibili REST API:
//! - Read: video, search, hot, ranking, user_videos, user_info, comments, feed, following, favorite, history, subtitle, me

use dashmap::DashMap;
use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::LazyLock;

mod api;
mod schema;
mod tools;

struct BilibiliTenant {
    sessdata: String,
    bili_jct: String,
    dede_user_id: String,
}

static BILIBILI_TENANTS: LazyLock<DashMap<String, BilibiliTenant>> =
    LazyLock::new(DashMap::new);

struct BilibiliPlugin;

impl Plugin for BilibiliPlugin {
    const NAME: &'static str = "bilibili";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        for tenant_config in &config.tenants {
            let name = tenant_config["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let sessdata = tenant_config["sessdata"].as_str().unwrap_or("").to_string();
            let bili_jct = tenant_config["bili_jct"].as_str().unwrap_or("").to_string();
            let dede_user_id = tenant_config["dede_user_id"].as_str().unwrap_or("").to_string();
            if sessdata.is_empty() {
                tracing::warn!(tenant = %name, "Skipping Bilibili tenant: missing sessdata");
                continue;
            }
            tracing::info!(tenant = %name, "Registered Bilibili tenant");
            BILIBILI_TENANTS.insert(name, BilibiliTenant { sessdata, bili_jct, dede_user_id });
        }
        tracing::info!("Bilibili plugin loaded with {} tenant(s)", BILIBILI_TENANTS.len());
        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ChannelAdapter>> {
        vec![]
    }

    fn tools(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ToolProvider>> {
        tools::build_all_tools()
    }
}

opencarrier_plugin_sdk::declare_plugin!(BilibiliPlugin);
