//! Plugin system types — host-side types not needed by plugins.
//!
//! Shared plugin types (PluginMessage, PluginConfig, etc.) live in
//! `opencarrier-plugin-sdk`. This module only contains host-side types.

use serde::{Deserialize, Serialize};

// Re-export shared types from the SDK for host-side convenience.
pub use opencarrier_plugin_sdk::{
    ChannelDescriptor, FfiJsonCallback, PluginConfig, PluginContent, PluginMessage, PluginMeta,
    PluginToolContext, PluginToolDef, PLUGIN_ABI_VERSION,
};

// ---------------------------------------------------------------------------
// Per-bot configuration (stored in <plugin-dir>/<bot-uuid>/bot.toml)
// ---------------------------------------------------------------------------

/// Per-bot configuration stored in `bot.toml`.
///
/// Each bot gets its own directory under the plugin directory:
/// `<plugin-dir>/<bot-uuid>/bot.toml`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    /// Human-readable bot name.
    pub name: String,
    /// Bot mode: "smartbot", "app", or "kf".
    #[serde(default)]
    pub mode: String,
    /// Bound agent ID (UUID string). None = unbound.
    #[serde(default)]
    pub bind_agent: Option<String>,
    /// Platform-specific fields stored as a flat TOML table.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Plugin status
// ---------------------------------------------------------------------------

/// Runtime status of a loaded plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatus {
    /// Plugin name.
    pub name: String,
    /// Plugin version.
    pub version: String,
    /// Whether the plugin is loaded and running.
    pub loaded: bool,
    /// Channel types provided.
    pub channels: Vec<String>,
    /// Tool names provided.
    pub tools: Vec<String>,
    /// Number of configured tenants.
    pub tenant_count: usize,
    /// Last error message (if any).
    #[serde(default)]
    pub last_error: Option<String>,
}
