//! Plugin tool dispatcher — routes tool calls to loaded plugins.

use std::sync::Arc;

use dashmap::DashMap;
use opencarrier_types::plugin::{PluginToolContext, PluginToolDef};
use opencarrier_types::tool::ToolDefinition;

use super::loader::LoadedPlugin;

// ---------------------------------------------------------------------------
// Tool entry
// ---------------------------------------------------------------------------

/// Entry mapping a tool name to its owning plugin.
struct PluginToolEntry {
    /// Name of the plugin providing this tool.
    plugin_name: String,
    /// The tool definition (description + parameter schema).
    definition: PluginToolDef,
    /// Reference to the loaded plugin (for execution).
    plugin: Arc<LoadedPlugin>,
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Dispatches plugin tool calls to the appropriate loaded plugin.
pub struct PluginToolDispatcher {
    tools: DashMap<String, PluginToolEntry>,
}

impl PluginToolDispatcher {
    /// Create a new empty dispatcher.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }

    /// Register all tools from a loaded plugin.
    pub fn register(&self, plugin: Arc<LoadedPlugin>) {
        for tool_def in &plugin.tools {
            let tool_name = tool_def.name.clone();
            self.tools.insert(
                tool_name,
                PluginToolEntry {
                    plugin_name: plugin.name.clone(),
                    definition: tool_def.clone(),
                    plugin: plugin.clone(),
                },
            );
        }
    }

    /// Unregister all tools from a specific plugin.
    pub fn unregister_plugin(&self, plugin_name: &str) {
        self.tools
            .retain(|_, entry| entry.plugin_name != plugin_name);
    }

    /// Check if a tool name is provided by any plugin.
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.contains_key(tool_name)
    }

    /// Get all plugin tool definitions (for LLM tool list).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .filter_map(|entry| {
                let schema: serde_json::Value =
                    serde_json::from_str(&entry.definition.parameters_json).ok()?;
                Some(ToolDefinition {
                    name: entry.definition.name.clone(),
                    description: entry.definition.description.clone(),
                    input_schema: schema,
                })
            })
            .collect()
    }

    /// Execute a plugin tool via C ABI.
    pub fn execute(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        context: &PluginToolContext,
    ) -> Result<String, String> {
        let entry = self
            .tools
            .get(tool_name)
            .ok_or_else(|| format!("Unknown plugin tool: {}", tool_name))?;

        let args_json =
            serde_json::to_string(args).map_err(|e| format!("Args serialization: {}", e))?;
        let context_json =
            serde_json::to_string(context).map_err(|e| format!("Context serialization: {}", e))?;

        entry
            .plugin
            .tool_execute(tool_name, &args_json, &context_json)
    }
}

impl Default for PluginToolDispatcher {
    fn default() -> Self {
        Self::new()
    }
}
