//! Plugin system types — shared between host and plugin crates.
//!
//! These types define the data that flows across the plugin ABI boundary.
//! All communication uses JSON serialization for ABI stability.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current plugin ABI version. Bumped on breaking changes.
pub const PLUGIN_ABI_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Plugin metadata
// ---------------------------------------------------------------------------

/// Plugin metadata from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    /// Unique plugin name (e.g. "wecom").
    pub name: String,
    /// Semantic version (e.g. "1.0.0").
    pub version: String,
    /// Minimum OpenCarrier version required.
    #[serde(default)]
    pub min_host_version: String,
    /// ABI version the plugin was compiled against.
    #[serde(default)]
    pub abi_version: u32,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Author name.
    #[serde(default)]
    pub author: String,
}

/// Full plugin configuration loaded from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Plugin metadata.
    #[serde(rename = "plugin")]
    pub meta: PluginMeta,
    /// Channel descriptors provided by this plugin.
    #[serde(default)]
    pub channels: Vec<ChannelDescriptor>,
    /// Tool descriptors provided by this plugin.
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    /// Schema for tenant configuration fields.
    #[serde(default)]
    pub config_schema: serde_json::Value,
    /// Legacy tenant configurations (from plugin.toml [[tenants]]).
    /// Empty after migration to per-bot bot.toml files.
    #[serde(default)]
    pub tenants: Vec<serde_json::Value>,
    /// Discovered bot configurations (from <plugin-dir>/<uuid>/bot.toml).
    /// Each value includes all bot config fields plus `_bot_id` (the bot UUID).
    #[serde(default)]
    pub bots: Vec<serde_json::Value>,
    /// Arbitrary plugin-specific configuration.
    #[serde(default)]
    pub extra: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Channel types
// ---------------------------------------------------------------------------

/// Descriptor for a channel provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDescriptor {
    /// Channel type identifier (e.g. "wecom", "telegram").
    pub channel_type: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: String,
}

/// Content types that can be exchanged with a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginContent {
    /// Plain text message.
    Text(String),
    /// Image with optional caption.
    Image {
        url: String,
        caption: Option<String>,
    },
    /// File attachment.
    File { url: String, filename: String },
    /// Voice message.
    Voice { url: String, duration_seconds: u32 },
    /// Geographic location.
    Location { lat: f64, lon: f64 },
    /// Bot command.
    Command { name: String, args: Vec<String> },
}

impl PluginContent {
    /// Extract text if this is a Text variant.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            PluginContent::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// A unified message from any channel plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMessage {
    /// Channel type this message came from.
    pub channel_type: String,
    /// Platform-specific message identifier.
    pub platform_message_id: String,
    /// Platform user ID of the sender.
    pub sender_id: String,
    /// Display name of the sender.
    #[serde(default)]
    pub sender_name: String,
    /// Tenant identifier (e.g. corp_id) — critical for multi-tenant routing.
    #[serde(default)]
    pub tenant_id: String,
    /// Message content.
    pub content: PluginContent,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Whether this message is from a group chat.
    #[serde(default)]
    pub is_group: bool,
    /// Thread ID for threaded conversations.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Arbitrary platform metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool types
// ---------------------------------------------------------------------------

/// A tool definition provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    /// Tool name (must be unique across all plugins).
    pub name: String,
    /// Description shown to the LLM.
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters_json: String,
}

/// Context provided when executing a plugin tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginToolContext {
    /// Tenant identifier — the plugin uses this to select credentials.
    #[serde(default)]
    pub tenant_id: String,
    /// Platform user ID of the message sender.
    #[serde(default)]
    pub sender_id: String,
    /// Agent ID that is calling the tool.
    #[serde(default)]
    pub agent_id: String,
    /// Channel type the message came from.
    #[serde(default)]
    pub channel_type: String,
}

// ---------------------------------------------------------------------------
// C ABI types (FFI-safe, JSON-based)
// ---------------------------------------------------------------------------

/// Callback function type: plugin sends a JSON-encoded PluginMessage to the host.
pub type FfiJsonCallback =
    unsafe extern "C" fn(user_data: *mut std::os::raw::c_void, json: *const std::os::raw::c_char);
