//! Built-in plugin — directly compiled channel adapters and tools (no FFI).
//!
//! Used for core channels (weixin, wecom, feishu) that ship with the binary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use opencarrier_plugin_sdk::ToolProvider;
use opencarrier_types::plugin::{PluginMessage, PluginToolDef};
use tokio::sync::mpsc;

use super::instance::PluginInstance;
use super::loader::LoadedChannel;

/// Trait for built-in channel adapters.
///
/// Similar to `opencarrier_plugin_sdk::ChannelAdapter` but uses the host's
/// native `mpsc::Sender<PluginMessage>` instead of an FFI callback.
pub trait BuiltinChannel: Send + Sync {
    /// Channel type identifier (e.g. "weixin").
    fn channel_type(&self) -> &str;

    /// Human-readable channel name.
    fn name(&self) -> &str;

    /// Tenant identifier this channel belongs to.
    fn tenant_id(&self) -> &str;

    /// Start receiving messages from the channel.
    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String>;

    /// Send a text message through the channel.
    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String>;

    /// Stop the channel and release resources.
    fn stop(&mut self);
}

/// A built-in plugin that directly holds Rust trait objects.
///
/// No FFI, no .so file — channels and tools are compiled into the main binary.
pub struct BuiltinPlugin {
    name: String,
    version: String,
    path: PathBuf,
    channels: Vec<LoadedChannel>,
    tools: Vec<PluginToolDef>,
    /// Channel adapters keyed by channel type.
    channel_adapters: Mutex<HashMap<String, Box<dyn BuiltinChannel>>>,
    /// Tool providers keyed by tool name.
    tool_providers: HashMap<String, Box<dyn ToolProvider>>,
}

impl BuiltinPlugin {
    /// Create a new built-in plugin.
    pub fn new(name: String, version: String, path: PathBuf) -> Self {
        Self {
            name,
            version,
            path,
            channels: Vec::new(),
            tools: Vec::new(),
            channel_adapters: Mutex::new(HashMap::new()),
            tool_providers: HashMap::new(),
        }
    }

    /// Register a channel adapter.
    pub fn register_channel(&mut self,
        mut adapter: Box<dyn BuiltinChannel>,
        sender: mpsc::Sender<PluginMessage>,
    ) -> Result<(), String> {
        let channel_type = adapter.channel_type().to_string();
        let name = adapter.name().to_string();
        let tenant_id = adapter.tenant_id().to_string();

        adapter.start(sender)?;

        self.channels.push(LoadedChannel {
            channel_type: channel_type.clone(),
            name,
            tenant_id,
            handle: std::ptr::null_mut(), // built-in: no opaque handle needed
        });

        self.channel_adapters.lock().unwrap().insert(channel_type, adapter);
        Ok(())
    }

    /// Register a tool provider.
    pub fn register_tool(&mut self,
        provider: Box<dyn ToolProvider>,
    ) {
        let def = provider.definition();
        self.tools.push(PluginToolDef {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters_json: def.parameters_json.clone(),
        });
        self.tool_providers.insert(def.name, provider);
    }
}

// SAFETY: BuiltinPlugin is Send+Sync because all fields are.
unsafe impl Send for BuiltinPlugin {}
unsafe impl Sync for BuiltinPlugin {}

impl PluginInstance for BuiltinPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn channels(&self) -> &[LoadedChannel] {
        &self.channels
    }

    fn tools(&self) -> &[PluginToolDef] {
        &self.tools
    }

    fn start_channel(&self, _channel: &LoadedChannel) -> Result<(), String> {
        // Channels are already started during registration.
        Ok(())
    }

    fn channel_send(
        &self,
        channel: &LoadedChannel,
        tenant_id: &str,
        user_id: &str,
        text: &str,
    ) -> Result<(), String> {
        let adapters = self.channel_adapters.lock().unwrap();
        if let Some(adapter) = adapters.get(&channel.channel_type) {
            adapter.send(tenant_id, user_id, text)
        } else {
            Err(format!("Built-in channel adapter '{}' not found", channel.channel_type))
        }
    }

    fn tool_execute(
        &self,
        tool_name: &str,
        args_json: &str,
        context_json: &str,
    ) -> Result<String, String> {
        let provider = self.tool_providers.get(tool_name)
            .ok_or_else(|| format!("Built-in tool '{}' not found", tool_name))?;

        let args: serde_json::Value = serde_json::from_str(args_json)
            .map_err(|e| format!("Args deserialization: {}", e))?;
        let ctx: opencarrier_types::plugin::PluginToolContext = serde_json::from_str(context_json)
            .map_err(|e| format!("Context deserialization: {}", e))?;

        provider.execute(&args, &ctx.into_sdk_context())
            .map_err(|e| e.to_string())
    }

    fn stop(&self) {
        let mut adapters = self.channel_adapters.lock().unwrap();
        for (name, adapter) in adapters.iter_mut() {
            adapter.stop();
            tracing::info!(channel = %name, "Built-in channel stopped");
        }
    }

    fn is_stopped(&self) -> bool {
        // Builtin plugins don't have an opaque handle; assume stopped if no adapters.
        self.channel_adapters.lock().unwrap().is_empty()
    }
}

/// Extension trait to convert runtime PluginToolContext to SDK context.
///
/// The SDK and runtime both define `PluginToolContext` with identical fields
/// (runtime re-exports from SDK via opencarrier-types). This trait bridges
/// the two type paths so built-in tools can use the SDK trait.
pub trait IntoSdkContext {
    fn into_sdk_context(self) -> opencarrier_plugin_sdk::PluginToolContext;
}

impl IntoSdkContext for opencarrier_types::plugin::PluginToolContext {
    fn into_sdk_context(self) -> opencarrier_plugin_sdk::PluginToolContext {
        // Since the types have identical layout (both come from the same source
        // in the SDK), serialize and deserialize is the safest bridge.
        serde_json::from_str(
            &serde_json::to_string(&self).unwrap_or_default()
        ).unwrap_or_default()
    }
}
