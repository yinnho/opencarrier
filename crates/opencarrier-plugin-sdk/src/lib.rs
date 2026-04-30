//! OpenCarrier Plugin SDK — write plugins in safe Rust.
//!
//! # Quick Start
//!
//! 1. Implement the [`Plugin`] trait for your plugin type.
//! 2. Optionally implement [`ChannelAdapter`] and [`ToolProvider`].
//! 3. Call [`declare_plugin!`](declare_plugin) at the bottom of `lib.rs`.
//! 4. Compile as `cdylib`.
//!
//! # Example
//!
//! ```ignore
//! use opencarrier_plugin_sdk::*;
//!
//! struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     const NAME: &'static str = "my-plugin";
//!     const VERSION: &'static str = "1.0.0";
//!
//!     fn new(_config: PluginConfig, _ctx: PluginContext) -> Result<Self, PluginError> {
//!         Ok(Self)
//!     }
//!
//!     fn tools(&self) -> Vec<Box<dyn ToolProvider>> {
//!         vec![Box::new(HelloTool)]
//!     }
//! }
//!
//! struct HelloTool;
//!
//! impl ToolProvider for HelloTool {
//!     fn definition(&self) -> ToolDef {
//!         ToolDef::new("hello", "Say hello", serde_json::json!({"type":"object"}))
//!     }
//!
//!     fn execute(&self, _args: &serde_json::Value, _ctx: &PluginToolContext) -> Result<String, PluginError> {
//!         Ok("Hello from plugin!".into())
//!     }
//! }
//!
//! declare_plugin!(MyPlugin);
//! ```
//!
//! # Cargo.toml for your plugin
//!
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! opencarrier-plugin-sdk = { path = "..." }
//! serde_json = "1"
//! ```

mod channel;
mod context;
mod error;
#[macro_use]
mod macros;
mod plugin;
mod tool;
mod types;

pub use channel::ChannelAdapter;
pub use context::{MessageSender, PluginContext};
pub use error::PluginError;
pub use plugin::Plugin;
pub use tool::{ToolDef, ToolProvider};

// Re-export plugin types from local types module
pub use types::{
    ChannelDescriptor, FfiJsonCallback, PluginConfig, PluginContent, PluginMessage, PluginMeta,
    PluginToolContext, PluginToolDef, PLUGIN_ABI_VERSION,
};
