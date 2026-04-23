//! Plugin trait — the main trait plugin developers implement.

use crate::channel::ChannelAdapter;
use crate::context::PluginContext;
use crate::error::PluginError;
use crate::tool::ToolProvider;
use opencarrier_types::plugin::PluginConfig;

/// The main plugin trait. Implement this for your plugin type.
///
/// # Example
///
/// ```ignore
/// struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     const NAME: &'static str = "my-plugin";
///     const VERSION: &'static str = "1.0.0";
///
///     fn new(config: PluginConfig, ctx: PluginContext) -> Result<Self, PluginError> {
///         Ok(Self)
///     }
///
///     fn tools(&self) -> Vec<Box<dyn ToolProvider>> {
///         vec![Box::new(MyTool)]
///     }
/// }
///
/// declare_plugin!(MyPlugin);
/// ```
pub trait Plugin: Send + Sync + 'static {
    /// Unique plugin name (must match plugin.toml).
    const NAME: &'static str;

    /// Semantic version string (e.g. "1.0.0").
    const VERSION: &'static str;

    /// Initialize the plugin with parsed configuration.
    fn new(config: PluginConfig, ctx: PluginContext) -> Result<Self, PluginError>
    where
        Self: Sized;

    /// Return channel adapters provided by this plugin.
    fn channels(&self) -> Vec<Box<dyn ChannelAdapter>> {
        Vec::new()
    }

    /// Return tool providers provided by this plugin.
    fn tools(&self) -> Vec<Box<dyn ToolProvider>> {
        Vec::new()
    }

    /// Called when the plugin is being unloaded. Default is a no-op.
    fn stop(&self) {}
}
