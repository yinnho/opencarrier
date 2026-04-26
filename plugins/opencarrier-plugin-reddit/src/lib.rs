//! Reddit platform plugin for OpenCarrier.
//!
//! Provides 15 tools via Reddit's REST API:
//! - Read: hot, frontpage, popular, subreddit, search, read, user,
//!   user_posts, user_comments, saved, upvoted
//! - Write: upvote, comment, save, subscribe

use dashmap::DashMap;
use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::LazyLock;

mod api;
mod schema;
mod tools;

// ---------------------------------------------------------------------------
// Global token store
// ---------------------------------------------------------------------------

/// Per-tenant Reddit credentials.
struct RedditTenant {
    cookie: String,
    username: Option<String>,
}

/// Global registry of Reddit tenants (keyed by tenant name).
static REDDIT_TENANTS: LazyLock<DashMap<String, RedditTenant>> =
    LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct RedditPlugin;

impl Plugin for RedditPlugin {
    const NAME: &'static str = "reddit";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        for tenant_config in &config.tenants {
            let name = tenant_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() {
                tracing::warn!("Skipping Reddit tenant with empty name");
                continue;
            }

            let cookie = tenant_config["cookie"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let username = tenant_config["username"]
                .as_str()
                .map(|s| s.to_string());

            if cookie.is_empty() {
                tracing::warn!(
                    tenant = %name,
                    "Skipping Reddit tenant: missing cookie"
                );
                continue;
            }

            tracing::info!(tenant = %name, "Registered Reddit tenant");
            REDDIT_TENANTS.insert(name, RedditTenant { cookie, username });
        }

        let tenant_count = REDDIT_TENANTS.len();
        if tenant_count == 0 {
            tracing::info!("Reddit plugin loaded (no tenants configured)");
        } else {
            tracing::info!("Reddit plugin loaded with {tenant_count} tenant(s)");
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

opencarrier_plugin_sdk::declare_plugin!(RedditPlugin);
