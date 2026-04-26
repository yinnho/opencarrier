//! Twitter/X platform plugin for OpenCarrier.
//!
//! Provides 20 tools via Twitter's GraphQL API:
//! - Read: search, timeline, tweets, profile, followers, following, thread,
//!         bookmarks, likes, lists, list_tweets, notifications, article
//! - Write: like, unlike, bookmark, follow, unfollow, post, delete

use dashmap::DashMap;
use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use std::sync::LazyLock;

mod api;
mod schema;
mod tools;

// ---------------------------------------------------------------------------
// Global token store
// ---------------------------------------------------------------------------

/// Per-tenant Twitter credentials.
struct TwitterTenant {
    ct0: String,
    auth_token: String,
}

/// Global registry of Twitter tenants (keyed by tenant name).
static TWITTER_TENANTS: LazyLock<DashMap<String, TwitterTenant>> =
    LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct TwitterPlugin;

impl Plugin for TwitterPlugin {
    const NAME: &'static str = "twitter";
    const VERSION: &'static str = "0.1.0";

    fn new(config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        for tenant_config in &config.tenants {
            let name = tenant_config["name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() {
                tracing::warn!("Skipping Twitter tenant with empty name");
                continue;
            }

            let ct0 = tenant_config["ct0"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let auth_token = tenant_config["auth_token"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if ct0.is_empty() || auth_token.is_empty() {
                tracing::warn!(
                    tenant = %name,
                    "Skipping Twitter tenant: missing ct0 or auth_token"
                );
                continue;
            }

            tracing::info!(tenant = %name, "Registered Twitter tenant");
            TWITTER_TENANTS.insert(name, TwitterTenant { ct0, auth_token });
        }

        let tenant_count = TWITTER_TENANTS.len();
        if tenant_count == 0 {
            tracing::info!("Twitter plugin loaded (no tenants configured)");
        } else {
            tracing::info!("Twitter plugin loaded with {tenant_count} tenant(s)");
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

opencarrier_plugin_sdk::declare_plugin!(TwitterPlugin);
