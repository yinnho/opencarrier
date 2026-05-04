//! Plugin manager — lifecycle management for loaded plugins.

use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use opencarrier_types::plugin::PluginMessage;
use opencarrier_types::tool::ToolDefinition;
use tokio::sync::mpsc;
use tracing::{error, info};

use super::bridge::PluginBridgeManager;
use super::builtin_registry::BuiltinPluginRegistry;
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
    loaded_plugins: Vec<Arc<dyn super::instance::PluginInstance>>,
    /// Kernel handle for bridge routing.
    kernel: Arc<dyn KernelHandle>,
    /// Shared channel bindings (kept after start() for dynamic additions).
    bridge_bindings: Option<Arc<DashMap<(String, String), String>>>,
    /// Shared tenant-to-bot map (kept after start() for dynamic additions).
    bridge_tenant_map: Option<Arc<DashMap<(String, String), String>>>,
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
            bridge_bindings: None,
            bridge_tenant_map: None,
        }
    }

    /// Load all plugins from the given directory.
    ///
    /// Loads external (.so) plugins and built-in plugins (compiled into the binary).
    /// Built-in plugins are identified by `builtin = true` in `plugin.toml` and
    /// constructed using the provided `BuiltinPluginRegistry`.
    pub fn load_all(&mut self, plugins_dir: &Path, registry: &BuiltinPluginRegistry) {
        // 1. Load external (.so) plugins
        let results = PluginLoader::load_all(plugins_dir, self.message_tx.clone());

        for result in results {
            match result {
                Ok(plugin) => {
                    let plugin_arc: Arc<dyn super::instance::PluginInstance> = Arc::new(plugin);
                    self.tool_dispatcher.register(plugin_arc.clone());
                    self.loaded_plugins.push(plugin_arc);
                }
                Err(e) => {
                    error!(error = %e, "Failed to load plugin");
                }
            }
        }

        // 2. Load built-in plugins
        let builtins = PluginLoader::load_builtin_plugins(
            plugins_dir,
            self.message_tx.clone(),
            registry,
        );
        for builtin in builtins {
            let plugin_arc: Arc<dyn super::instance::PluginInstance> = Arc::new(builtin);
            self.tool_dispatcher.register(plugin_arc.clone());
            self.loaded_plugins.push(plugin_arc);
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
            for channel in plugin.channels() {
                if let Err(e) = plugin.start_channel(channel) {
                    error!(
                        plugin = %plugin.name(),
                        channel = %channel.channel_type,
                        error = %e,
                        "Failed to start channel"
                    );
                } else {
                    info!(
                        plugin = %plugin.name(),
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
            let plugin_dir = plugin.path();
            let bots = super::loader::PluginLoader::discover_bots(plugin_dir);
            let channels: Vec<String> = plugin
                .channels()
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

                    // Also bind using channel descriptor tenant_id (may differ from bot_uuid).
                    // Plugins like weixin/feishu use the tenant name as tenant_id in messages.
                    for channel in plugin.channels() {
                        let ch_type = &channel.channel_type;
                        if ch_type != "weixin" && ch_type != "feishu" {
                            continue;
                        }
                        if !channel.tenant_id.is_empty() && channel.tenant_id != *bot_uuid {
                            bridge.bind_channel(
                                channel.channel_type.clone(),
                                channel.tenant_id.clone(),
                                agent_uuid.clone(),
                            );
                            bridge.map_channel_tenant(
                                channel.channel_type.clone(),
                                channel.tenant_id.clone(),
                                bot_uuid.clone(),
                            );
                            info!(
                                channel = %channel.channel_type,
                                tenant_id = %channel.tenant_id,
                                agent_id = %agent_uuid,
                                "Bound channel tenant_id to agent"
                            );
                        }
                    }

                    // Feishu bots use the bot name as tenant_id in messages, but the watcher
                    // channel has empty tenant_id. Bind using the bot name directly.
                    if channels.contains(&"feishu".to_string()) {
                        let tenant_name = &bot_config.name;
                        if tenant_name != bot_uuid {
                            bridge.bind_channel(
                                "feishu".to_string(),
                                tenant_name.clone(),
                                agent_uuid.clone(),
                            );
                            bridge.map_channel_tenant(
                                "feishu".to_string(),
                                tenant_name.clone(),
                                bot_uuid.clone(),
                            );
                            info!(
                                tenant_name = %tenant_name,
                                agent_id = %agent_uuid,
                                "Bound feishu bot name to agent"
                            );
                        }
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

        // Keep shared references for dynamic binding additions
        self.bridge_bindings = Some(bridge.shared_bindings());
        self.bridge_tenant_map = Some(bridge.shared_tenant_map());

        // Start bridge in a background task
        if let Some(rx) = self.message_rx.take() {
            tokio::spawn(async move {
                bridge.run(rx).await;
            });
        }
    }

    /// Dynamically add a channel binding (e.g., when a bot is bound to an agent via API).
    /// This takes effect immediately without restarting the bridge.
    pub fn add_channel_binding(&self, channel_type: &str, key: &str, agent_id: &str) {
        if let Some(ref bindings) = self.bridge_bindings {
            bindings.insert(
                (channel_type.to_string(), key.to_string()),
                agent_id.to_string(),
            );
            info!(
                channel = %channel_type,
                key = %key,
                agent = %agent_id,
                "Dynamically bound channel to agent"
            );
        }
    }

    /// Dynamically map a channel's tenant_id to its bot UUID.
    pub fn map_channel_tenant(&self, channel_type: &str, tenant_id: &str, bot_uuid: &str) {
        if let Some(ref map) = self.bridge_tenant_map {
            map.insert(
                (channel_type.to_string(), tenant_id.to_string()),
                bot_uuid.to_string(),
            );
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
                name: p.name().to_string(),
                version: p.version().to_string(),
                loaded: true,
                channels: p.channels().iter().map(|c| c.channel_type.clone()).collect(),
                tools: p.tools().iter().map(|t| t.name.clone()).collect(),
                tenant_count: 0,
                last_error: None,
            })
            .collect()
    }

    /// Stop all plugins and release resources.
    pub fn stop_all(&self) {
        for plugin in &self.loaded_plugins {
            info!(plugin = %plugin.name(), "Stopping plugin");
            plugin.stop();
        }
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}
