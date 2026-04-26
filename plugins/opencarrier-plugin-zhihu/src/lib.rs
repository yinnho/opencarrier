//! Zhihu platform plugin for OpenCarrier.
//!
//! Provides 3 tools via Zhihu REST API:
//! - hot, question, search

use dashmap::DashMap;
use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::LazyLock;

mod api;
mod schema;
mod tools;

struct ZhihuTenant {
    cookie: String,
}

static ZHIHU_TENANTS: LazyLock<DashMap<String, ZhihuTenant>> =
    LazyLock::new(DashMap::new);

struct ZhihuPlugin;

impl Plugin for ZhihuPlugin {
    const NAME: &'static str = "zhihu";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        for tenant_config in &config.tenants {
            let name = tenant_config["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let cookie = tenant_config["cookie"].as_str().unwrap_or("").to_string();
            if cookie.is_empty() {
                tracing::warn!(tenant = %name, "Skipping Zhihu tenant: missing cookie");
                continue;
            }
            tracing::info!(tenant = %name, "Registered Zhihu tenant");
            ZHIHU_TENANTS.insert(name, ZhihuTenant { cookie });
        }
        tracing::info!("Zhihu plugin loaded with {} tenant(s)", ZHIHU_TENANTS.len());
        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ChannelAdapter>> {
        vec![]
    }

    fn tools(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ToolProvider>> {
        tools::build_all_tools()
    }
}

opencarrier_plugin_sdk::declare_plugin!(ZhihuPlugin);
