//! Plugin bridge — routes messages between plugin channels and the kernel.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use opencarrier_types::plugin::PluginMessage;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::instance::PluginInstance;
use crate::kernel_handle::KernelHandle;

// ---------------------------------------------------------------------------
// Bridge manager
// ---------------------------------------------------------------------------

/// Routes inbound plugin messages to agents and delivers responses back
/// through the originating channel.
pub struct PluginBridgeManager {
    /// Kernel handle for sending messages to agents.
    kernel: Arc<dyn KernelHandle>,
    /// Loaded plugins (for channel_send responses and plugin directory paths).
    plugins: Vec<Arc<dyn PluginInstance>>,
    /// (channel_type, bot_id) → agent_id bindings.
    /// Shared via Arc so PluginManager can add dynamic bindings.
    channel_bindings: Arc<DashMap<(String, String), String>>,
    /// Maps (channel_type, channel_tenant_id) → bot UUID for finding bot.toml.
    channel_tenant_to_bot_uuid: Arc<DashMap<(String, String), String>>,
    /// Bot IDs that already have an owner set (avoids repeated file reads).
    owned_bots: Arc<Mutex<HashSet<String>>>,
}

impl PluginBridgeManager {
    /// Create a new bridge manager.
    pub fn new(kernel: Arc<dyn KernelHandle>) -> Self {
        Self {
            kernel,
            plugins: Vec::new(),
            channel_bindings: Arc::new(DashMap::new()),
            channel_tenant_to_bot_uuid: Arc::new(DashMap::new()),
            owned_bots: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Get a shared reference to the channel bindings map.
    pub fn shared_bindings(&self) -> Arc<DashMap<(String, String), String>> {
        Arc::clone(&self.channel_bindings)
    }

    /// Get a shared reference to the tenant-to-bot-UUID map.
    pub fn shared_tenant_map(&self) -> Arc<DashMap<(String, String), String>> {
        Arc::clone(&self.channel_tenant_to_bot_uuid)
    }

    /// Add a loaded plugin to the bridge.
    pub fn add_plugin(&mut self, plugin: Arc<dyn PluginInstance>) {
        self.plugins.push(plugin);
    }

    /// Bind a specific (channel_type, bot_id) to an agent.
    pub fn bind_channel(&mut self, channel_type: String, bot_id: String, agent_id: String) {
        info!(
            channel = %channel_type,
            bot = %bot_id,
            agent = %agent_id,
            "Bound channel+bot to agent"
        );
        self.channel_bindings
            .insert((channel_type, bot_id), agent_id);
    }

    /// Map a channel's tenant_id to its bot UUID (for finding bot.toml on disk).
    pub fn map_channel_tenant(&mut self, channel_type: String, tenant_id: String, bot_uuid: String) {
        self.channel_tenant_to_bot_uuid
            .insert((channel_type, tenant_id), bot_uuid);
    }

    /// Remove a (channel_type, bot_id) binding.
    pub fn unbind_channel(&mut self, channel_type: &str, bot_id: &str) {
        self.channel_bindings
            .remove(&(channel_type.to_string(), bot_id.to_string()));
    }

    /// Remove a channel tenant mapping.
    pub fn unmap_channel_tenant(&mut self, channel_type: &str, tenant_id: &str) {
        self.channel_tenant_to_bot_uuid
            .remove(&(channel_type.to_string(), tenant_id.to_string()));
    }

    /// Mark a bot as already having an owner (called at startup).
    pub fn mark_bot_owned(&mut self, bot_id: String) {
        self.owned_bots.lock().unwrap().insert(bot_id);
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
        // Set owner on first message
        self.try_set_owner(&msg).await;

        let text = match msg.content.as_text() {
            Some(t) => t.to_string(),
            None => self.describe_non_text_content(&msg),
        };

        // Route by (channel_type, bot_id)
        let agent_id = match self
            .channel_bindings
            .get(&(msg.channel_type.clone(), msg.tenant_id.clone()))
        {
            Some(id) => id.clone(),
            None => {
                warn!(
                    channel = %msg.channel_type,
                    bot = %msg.tenant_id,
                    "No agent binding for channel+bot, dropping message"
                );
                return;
            }
        };

        info!(
            channel = %msg.channel_type,
            tenant = %msg.tenant_id,
            agent = %agent_id,
            sender = %msg.sender_name,
            "Routing plugin message to agent"
        );

        match self
            .kernel
            .send_to_agent(
                &agent_id,
                &text,
                Some(&msg.sender_id),
                Some(&msg.sender_name),
                None,
                Some(&msg.tenant_id),
            )
            .await
        {
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

    /// If this bot has no owner yet, set the message sender as owner.
    async fn try_set_owner(&self, msg: &PluginMessage) {
        let bot_id = &msg.tenant_id;
        if bot_id.is_empty() || msg.sender_id.is_empty() {
            return;
        }

        {
            let owned = self.owned_bots.lock().unwrap();
            if owned.contains(bot_id) {
                return;
            }
        }

        // Find the plugin directory for this bot
        let bot_toml_path = match self.find_bot_toml(&msg.channel_type, bot_id) {
            Some(p) => p,
            None => return,
        };

        match write_owner_id(&bot_toml_path, &msg.sender_id) {
            Ok(()) => {
                info!(
                    bot = %bot_id,
                    owner = %msg.sender_id,
                    "Set bot owner (first message)"
                );
                self.owned_bots.lock().unwrap().insert(bot_id.clone());
            }
            Err(e) => {
                warn!(
                    bot = %bot_id,
                    error = %e,
                    "Failed to write owner_id to bot.toml"
                );
            }
        }
    }

    /// Find the bot.toml path for a given (channel_type, tenant_id).
    fn find_bot_toml(&self, channel_type: &str, tenant_id: &str) -> Option<std::path::PathBuf> {
        // Resolve tenant_id to bot UUID via mapping, or use tenant_id directly
        let resolved = self
            .channel_tenant_to_bot_uuid
            .get(&(channel_type.to_string(), tenant_id.to_string()))
            .map(|r| r.value().clone());
        let dir_name = resolved.as_deref().unwrap_or(tenant_id);

        for plugin in &self.plugins {
            for channel in plugin.channels() {
                if channel.channel_type == channel_type && channel.tenant_id == tenant_id {
                    let path = plugin.path().join("bot").join(dir_name).join("bot.toml");
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
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
                format!("[用户发送了一段{}秒的语音]: {}", duration_seconds, url)
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
        // Try exact match first (tenant_id matches a specific LoadedChannel)
        for plugin in &self.plugins {
            for channel in plugin.channels() {
                if channel.channel_type == original.channel_type
                    && channel.tenant_id == original.tenant_id
                {
                    if let Err(e) = plugin.channel_send(
                        channel,
                        &original.tenant_id,
                        &original.sender_id,
                        response,
                    ) {
                        error!(
                            channel = %channel.channel_type,
                            tenant = %channel.tenant_id,
                            error = %e,
                            "Failed to send response through channel"
                        );
                    }
                    return;
                }
            }
        }
        // Fallback: any channel of the same type handles dynamic tenants.
        // The channel adapter's send() looks up the tenant in its own state.
        for plugin in &self.plugins {
            for channel in plugin.channels() {
                if channel.channel_type == original.channel_type {
                    if let Err(e) = plugin.channel_send(
                        channel,
                        &original.tenant_id,
                        &original.sender_id,
                        response,
                    ) {
                        error!(
                            channel = %channel.channel_type,
                            tenant = %original.tenant_id,
                            error = %e,
                            "Failed to send response through fallback channel"
                        );
                    }
                    return;
                }
            }
        }
        warn!(
            channel = %original.channel_type,
            tenant = %original.tenant_id,
            "No plugin channel found for response"
        );
    }
}

// ---------------------------------------------------------------------------
// bot.toml owner write helper
// ---------------------------------------------------------------------------

/// Write `owner_id` into a bot.toml file (read → parse → insert → write).
fn write_owner_id(path: &Path, owner_id: &str) -> Result<(), String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let mut doc = content
        .parse::<toml::Value>()
        .map_err(|e| format!("解析失败: {e}"))?;
    let table = doc
        .as_table_mut()
        .ok_or("Invalid bot.toml structure".to_string())?;
    table.insert(
        "owner_id".into(),
        toml::Value::String(owner_id.to_string()),
    );
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("序列化失败: {e}"))?;
    // Atomic write: write to tmp file then rename
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &new_content).map_err(|e| format!("写入临时文件失败: {e}"))?;
    std::fs::rename(&tmp_path, path).map_err(|e| format!("重命名失败: {e}"))?;
    Ok(())
}
