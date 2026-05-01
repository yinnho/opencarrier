//! Plugin manager — lifecycle management for loaded plugins.

use std::path::Path;
use std::sync::Arc;

use opencarrier_types::plugin::PluginMessage;
use opencarrier_types::tool::ToolDefinition;
use tokio::sync::mpsc;
use tracing::{error, info};

use super::bridge::PluginBridgeManager;
use super::loader::PluginLoader;
use super::tool_dispatch::PluginToolDispatcher;
use crate::kernel_handle::KernelHandle;

// ---------------------------------------------------------------------------
// Plugin manager
// ---------------------------------------------------------------------------

/// Manages the lifecycle of all loaded plugins: loading, starting, stopping.
pub struct PluginManager {
    /// Tool dispatcher for routing tool calls.
    tool_dispatcher: Arc<PluginToolDispatcher>,
    /// Bridge message sender (inbound messages from plugins).
    message_tx: mpsc::Sender<PluginMessage>,
    /// Bridge message receiver (moved to bridge on start).
    message_rx: Option<mpsc::Receiver<PluginMessage>>,
    /// Successfully loaded plugins.
    loaded_plugins: Vec<Arc<super::loader::LoadedPlugin>>,
    /// Kernel handle for bridge routing.
    kernel: Arc<dyn KernelHandle>,
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new(kernel: Arc<dyn KernelHandle>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            tool_dispatcher: Arc::new(PluginToolDispatcher::new()),
            message_tx: tx,
            message_rx: Some(rx),
            loaded_plugins: Vec::new(),
            kernel,
        }
    }

    /// Load all plugins from the given directory.
    ///
    /// Each subdirectory should contain `plugin.toml` and a shared library.
    pub fn load_all(&mut self, plugins_dir: &Path) {
        let results = PluginLoader::load_all(plugins_dir, self.message_tx.clone());

        for result in results {
            match result {
                Ok(plugin) => {
                    let plugin_arc = Arc::new(plugin);
                    self.tool_dispatcher.register(plugin_arc.clone());
                    self.loaded_plugins.push(plugin_arc);
                }
                Err(e) => {
                    error!(error = %e, "Failed to load plugin");
                }
            }
        }

        info!(
            loaded = self.loaded_plugins.len(),
            tools = self.tool_dispatcher.definitions().len(),
            "Plugin loading complete"
        );
    }

    /// Start all channel adapters and the bridge.
    ///
    /// Discovers bot configs from `<plugin>/<uuid>/bot.toml` files and binds
    /// channels to agents based on `bind_agent` (agent UUID) in each bot config.
    pub async fn start(&mut self, _plugins_dir: &Path) {
        // Start channel adapters
        for plugin in &self.loaded_plugins {
            for channel in &plugin.channels {
                if let Err(e) = plugin.start_channel(channel) {
                    error!(
                        plugin = %plugin.name,
                        channel = %channel.channel_type,
                        error = %e,
                        "Failed to start channel"
                    );
                } else {
                    info!(
                        plugin = %plugin.name,
                        channel = %channel.channel_type,
                        "Channel started"
                    );
                }
            }
        }

        // Build channel bindings from bot.toml files
        let mut bridge = PluginBridgeManager::new(self.kernel.clone());
        for plugin in &self.loaded_plugins {
            bridge.add_plugin(plugin.clone());

            // Discover bots from <plugin-dir>/<uuid>/bot.toml
            let plugin_dir = &plugin.path;
            let bots = super::loader::PluginLoader::discover_bots(plugin_dir);
            let channels: Vec<String> = plugin
                .channels
                .iter()
                .map(|c| c.channel_type.clone())
                .collect();

            for (bot_uuid, bot_config) in &bots {
                // Mark bots that already have an owner
                if bot_config.owner_id.is_some() {
                    bridge.mark_bot_owned(bot_uuid.clone());
                }

                if let Some(ref agent_uuid) = bot_config.bind_agent {
                    // bind_agent must be a UUID — agent names are not unique across tenants
                    if uuid::Uuid::parse_str(agent_uuid).is_err() {
                        error!(
                            bot = %bot_config.name,
                            bind_agent = %agent_uuid,
                            "bind_agent is not a valid UUID, skipping binding"
                        );
                        continue;
                    }

                    for ch in &channels {
                        bridge.bind_channel(ch.clone(), bot_uuid.clone(), agent_uuid.clone());
                        info!(
                            channel = %ch,
                            bot = %bot_config.name,
                            bot_id = %bot_uuid,
                            agent_id = %agent_uuid,
                            "Bound bot to agent"
                        );
                    }

                    // Set default plugin tenant for the agent (used when no channel context)
                    self.kernel.set_default_plugin_tenant(agent_uuid, bot_uuid);
                } else {
                    info!(
                        bot = %bot_config.name,
                        bot_id = %bot_uuid,
                        "Bot has no bind_agent, skipping"
                    );
                }
            }
        }

        // Start bridge in a background task
        if let Some(rx) = self.message_rx.take() {
            tokio::spawn(async move {
                bridge.run(rx).await;
            });
        }
    }

    /// Get all plugin tool definitions (for the LLM tool list).
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_dispatcher.definitions()
    }

    /// Get a reference to the tool dispatcher (for execute_tool integration).
    pub fn tool_dispatcher(&self) -> Arc<PluginToolDispatcher> {
        self.tool_dispatcher.clone()
    }

    /// Get status of all loaded plugins.
    pub fn status(&self) -> Vec<opencarrier_types::plugin::PluginStatus> {
        self.loaded_plugins
            .iter()
            .map(|p| opencarrier_types::plugin::PluginStatus {
                name: p.name.clone(),
                version: p.version.clone(),
                loaded: true,
                channels: p.channels.iter().map(|c| c.channel_type.clone()).collect(),
                tools: p.tools.iter().map(|t| t.name.clone()).collect(),
                tenant_count: 0,
                last_error: None,
            })
            .collect()
    }

    /// Stop all plugins and release resources.
    pub fn stop_all(&self) {
        for plugin in &self.loaded_plugins {
            info!(plugin = %plugin.name, "Stopping plugin");
            plugin.stop();
        }
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}
