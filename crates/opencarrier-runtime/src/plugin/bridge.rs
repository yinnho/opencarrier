//! Plugin bridge — routes messages between plugin channels and the kernel.

use std::collections::HashMap;
use std::sync::Arc;

use opencarrier_types::plugin::PluginMessage;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::kernel_handle::KernelHandle;
use super::loader::LoadedPlugin;

// ---------------------------------------------------------------------------
// Bridge manager
// ---------------------------------------------------------------------------

/// Routes inbound plugin messages to agents and delivers responses back
/// through the originating channel.
pub struct PluginBridgeManager {
    /// Kernel handle for sending messages to agents.
    kernel: Arc<dyn KernelHandle>,
    /// Loaded plugins (for channel_send responses).
    plugins: Vec<Arc<LoadedPlugin>>,
    /// Default agent ID for routing when no specific binding exists.
    default_agent_id: Option<String>,
    /// Channel type → agent ID bindings.
    channel_bindings: HashMap<String, String>,
}

impl PluginBridgeManager {
    /// Create a new bridge manager.
    pub fn new(kernel: Arc<dyn KernelHandle>) -> Self {
        Self {
            kernel,
            plugins: Vec::new(),
            default_agent_id: None,
            channel_bindings: HashMap::new(),
        }
    }

    /// Add a loaded plugin to the bridge.
    pub fn add_plugin(&mut self, plugin: Arc<LoadedPlugin>) {
        self.plugins.push(plugin);
    }

    /// Set the default agent for routing unbound messages.
    pub fn set_default_agent(&mut self, agent_id: String) {
        self.default_agent_id = Some(agent_id);
    }

    /// Bind a channel type to a specific agent.
    pub fn bind_channel(&mut self, channel_type: String, agent_id: String) {
        self.channel_bindings.insert(channel_type, agent_id);
    }

    /// Run the message processing loop (consumes self).
    pub async fn run(self, mut rx: mpsc::Receiver<PluginMessage>) {
        info!("Plugin bridge started");

        while let Some(msg) = rx.recv().await {
            self.handle_inbound(msg).await;
        }

        info!("Plugin bridge stopped (channel closed)");
    }

    // -----------------------------------------------------------------------
    // Inbound message handling
    // -----------------------------------------------------------------------

    async fn handle_inbound(&self, msg: PluginMessage) {
        let text = match msg.content.as_text() {
            Some(t) => t.to_string(),
            None => self.describe_non_text_content(&msg),
        };

        // Route to agent
        let agent_id = self
            .channel_bindings
            .get(&msg.channel_type)
            .cloned()
            .or_else(|| self.default_agent_id.clone());

        let agent_id = match agent_id {
            Some(id) => id,
            None => {
                warn!(
                    channel = %msg.channel_type,
                    "No agent binding for channel, dropping message"
                );
                return;
            }
        };

        info!(
            channel = %msg.channel_type,
            agent = %agent_id,
            sender = %msg.sender_name,
            tenant = %msg.tenant_id,
            "Routing plugin message to agent"
        );

        match self.kernel.send_to_agent(&agent_id, &text, Some(&msg.sender_id), Some(&msg.sender_name), None).await {
            Ok(response) => {
                self.send_response(&msg, &response);
            }
            Err(e) => {
                error!(
                    agent = %agent_id,
                    error = %e,
                    "Failed to send message to agent"
                );
            }
        }
    }

    fn describe_non_text_content(&self, msg: &PluginMessage) -> String {
        use opencarrier_types::plugin::PluginContent;
        match &msg.content {
            PluginContent::Image { url, caption } => {
                let cap = caption
                    .as_deref()
                    .map(|c| format!(" ({})", c))
                    .unwrap_or_default();
                format!("[用户发送了一张图片{}]: {}", cap, url)
            }
            PluginContent::File { url, filename } => {
                format!("[用户发送了一个文件]: {} ({})", filename, url)
            }
            PluginContent::Voice {
                url,
                duration_seconds,
            } => {
                format!(
                    "[用户发送了一段{}秒的语音]: {}",
                    duration_seconds, url
                )
            }
            PluginContent::Location { lat, lon } => {
                format!("[用户发送了位置]: 经度 {}, 纬度 {}", lon, lat)
            }
            PluginContent::Command { name, args } => {
                format!("[用户发送了命令]: {} {:?}", name, args)
            }
            PluginContent::Text(_) => unreachable!(),
        }
    }

    // -----------------------------------------------------------------------
    // Outbound response
    // -----------------------------------------------------------------------

    fn send_response(&self, original: &PluginMessage, response: &str) {
        for plugin in &self.plugins {
            for channel in &plugin.channels {
                if channel.channel_type == original.channel_type {
                    if let Err(e) = plugin.channel_send(
                        channel,
                        &original.tenant_id,
                        &original.sender_id,
                        response,
                    ) {
                        error!(
                            channel = %channel.channel_type,
                            error = %e,
                            "Failed to send response through channel"
                        );
                    }
                    return;
                }
            }
        }
        warn!(
            channel = %original.channel_type,
            "No plugin channel found for response"
        );
    }
}
