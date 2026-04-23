//! Channel adapter trait — for plugins that bridge external messaging platforms.

use crate::context::MessageSender;
use crate::error::PluginError;

/// A channel adapter bridges an external messaging platform to OpenCarrier.
///
/// Implement this trait for each channel type your plugin supports
/// (e.g. WeCom, Telegram, Discord).
///
/// # Lifecycle
///
/// 1. The host calls `channel_type()` and `name()` during loading.
/// 2. The host calls `start(sender)` to begin receiving messages.
/// 3. The adapter calls `sender.send(msg)` when inbound messages arrive.
/// 4. The host calls `send(tenant_id, user_id, text)` for outbound replies.
/// 5. The host calls `stop()` when shutting down.
pub trait ChannelAdapter: Send + Sync {
    /// Channel type identifier (e.g. "wecom", "telegram").
    fn channel_type(&self) -> &str;

    /// Human-readable channel name.
    fn name(&self) -> &str;

    /// Start receiving messages from the channel.
    ///
    /// Store the `sender` — call `sender.send(msg)` for each inbound message.
    fn start(&mut self, sender: MessageSender) -> Result<(), PluginError>;

    /// Send a text message through the channel.
    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), PluginError>;

    /// Stop the channel and release resources. Default is a no-op.
    fn stop(&mut self) {}
}
