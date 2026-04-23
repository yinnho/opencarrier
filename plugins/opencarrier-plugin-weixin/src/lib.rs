//! WeChat personal account (iLink Bot) plugin for OpenCarrier.
//!
//! Provides a long-polling channel adapter that receives and replies to
//! WeChat messages via Tencent's official iLink Bot API.
//!
//! **Flow**: QR code scan → 24h bot_token → long-poll getupdates → reply with context_token.

// Phase 1 only uses text send/receive; typing/media types are kept for Phase 2.
#![allow(dead_code)]

use opencarrier_plugin_sdk::{Plugin, PluginConfig, PluginContext, PluginError};
use token::WEIXIN_STATE;

mod api;
mod channel;
mod token;
mod types;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct WeixinPlugin;

impl Plugin for WeixinPlugin {
    const NAME: &'static str = "weixin";
    const VERSION: &'static str = "0.1.0";

    fn new(_config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
        // Load persisted tokens from disk (non-expired ones only)
        let token_dir = WEIXIN_STATE.token_dir.clone();
        WEIXIN_STATE.load_from_dir(&token_dir);

        let active = WEIXIN_STATE.active_tenant_names();
        if active.is_empty() {
            tracing::info!("WeChat iLink plugin loaded (no active tenants, bind via Dashboard QR scan)");
        } else {
            tracing::info!(
                tenants = ?active,
                "WeChat iLink plugin loaded"
            );
        }

        Ok(Self)
    }

    fn channels(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ChannelAdapter>> {
        WEIXIN_STATE
            .active_tenant_names()
            .into_iter()
            .map(|name| {
                let ch = channel::ILinkChannel::new(name);
                Box::new(ch) as Box<dyn opencarrier_plugin_sdk::ChannelAdapter>
            })
            .collect()
    }

    fn tools(&self) -> Vec<Box<dyn opencarrier_plugin_sdk::ToolProvider>> {
        vec![]
    }
}

opencarrier_plugin_sdk::declare_plugin!(WeixinPlugin);
