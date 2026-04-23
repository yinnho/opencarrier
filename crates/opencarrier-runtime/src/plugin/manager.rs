//! Plugin manager — lifecycle management for loaded plugins.

use std::path::Path;
use std::sync::Arc;

use opencarrier_types::plugin::PluginMessage;
use opencarrier_types::tool::ToolDefinition;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::kernel_handle::KernelHandle;
use super::bridge::PluginBridgeManager;
use super::loader::PluginLoader;
use super::tool_dispatch::PluginToolDispatcher;

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
    /// Reads `bind_agent` from tenant configs in plugin.toml to route
    /// channel messages to specific agents.
    pub async fn start(&mut self, plugins_dir: &Path) {
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

        // Build channel bindings from tenant configs
        let mut bridge = PluginBridgeManager::new(self.kernel.clone());
        for plugin in &self.loaded_plugins {
            bridge.add_plugin(plugin.clone());
        }

        // Read plugin.toml files for bind_agent configuration
        if let Ok(entries) = std::fs::read_dir(plugins_dir) {
            let agents = self.kernel.list_agents();
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let toml_path = entry.path().join("plugin.toml");
                    if toml_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&toml_path) {
                            if let Ok(config) = toml::from_str::<opencarrier_types::plugin::PluginConfig>(&content) {
                                // Determine channel_type from mode
                                let channels: Vec<&str> = config.channels.iter().map(|c| c.channel_type.as_str()).collect();
                                for tenant in &config.tenants {
                                    if let Some(agent_name) = tenant.get("bind_agent").and_then(|v| v.as_str()) {
                                        // Find agent ID by name
                                        if let Some(agent) = agents.iter().find(|a| a.name == agent_name) {
                                            // Bind each channel from this plugin to the agent
                                            for ch in &channels {
                                                bridge.bind_channel(ch.to_string(), agent.id.clone());
                                                info!(
                                                    channel = %ch,
                                                    agent = %agent_name,
                                                    agent_id = %agent.id,
                                                    "Bound channel to agent"
                                                );
                                            }
                                        } else {
                                            error!(agent = %agent_name, "bind_agent not found, skipping binding");
                                        }
                                    }
                                }
                            }
                        }
                    }
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
