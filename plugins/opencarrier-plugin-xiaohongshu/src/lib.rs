//! Xiaohongshu Creator API plugin for OpenCarrier.
//!
//! Provides 5 tools via Xiaohongshu Creator API (cookie auth):
//! - xhs_creator_notes: 创作者笔记列表
//! - xhs_creator_note_detail: 单篇笔记详情
//! - xhs_creator_profile: 创作者账号信息
//! - xhs_creator_stats: 数据总览
//! - xhs_creator_notes_summary: 笔记批量摘要

use dashmap::DashMap;
use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::LazyLock;

mod api;
mod schema;
mod tools;

// ---------------------------------------------------------------------------
// Global cookie store
// ---------------------------------------------------------------------------

/// Per-tenant Xiaohongshu credentials.
struct XhsTenant {
    cookie: String,
}

/// Global registry of Xiaohongshu tenants (keyed by tenant name).
static XHS_TENANTS: LazyLock<DashMap<String, XhsTenant>> =
    LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct XiaohongshuPlugin;

impl Plugin for XiaohongshuPlugin {
    const NAME: &'static str = "xiaohongshu";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        for tenant_config in &config.tenants {
            let name = tenant_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() {
                tracing::warn!("Skipping Xiaohongshu tenant with empty name");
                continue;
            }

            let cookie = tenant_config["cookie"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if cookie.is_empty() {
                tracing::warn!(
                    tenant = %name,
                    "Skipping Xiaohongshu tenant: missing cookie"
                );
                continue;
            }

            tracing::info!(tenant = %name, "Registered Xiaohongshu tenant");
            XHS_TENANTS.insert(name, XhsTenant { cookie });
        }

        let tenant_count = XHS_TENANTS.len();
        if tenant_count == 0 {
            tracing::info!("Xiaohongshu plugin loaded (no tenants configured)");
        } else {
            tracing::info!("Xiaohongshu plugin loaded with {tenant_count} tenant(s)");
        }

        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ChannelAdapter>> {
        vec![]
    }

    fn tools(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ToolProvider>> {
        tools::build_all_tools()
    }
}

opencarrier_plugin_sdk::declare_plugin!(XiaohongshuPlugin);
