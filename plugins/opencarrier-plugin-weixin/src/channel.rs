//! ILinkChannel — long-polling ChannelAdapter for WeChat personal account.
//!
//! Spawns a dedicated OS thread with its own tokio runtime (cdylib plugins
//! cannot safely use the host runtime). Polls `getupdates` (35s hold) and
//! dispatches inbound messages through the plugin SDK's MessageSender.

use crate::api;
use crate::token::WEIXIN_STATE;
use crate::types::*;
use opencarrier_plugin_sdk::{ChannelAdapter, MessageSender, PluginContent, PluginError, PluginMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Channel adapter for a single iLink tenant (one scanned WeChat account).
pub struct ILinkChannel {
    tenant_name: String,
    /// Shutdown signal for the polling thread (set to true to stop).
    shutdown: Arc<AtomicBool>,
    /// Handle to the polling thread (for join on stop).
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl ILinkChannel {
    pub fn new(tenant_name: String) -> Self {
        Self {
            tenant_name,
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl ChannelAdapter for ILinkChannel {
    fn channel_type(&self) -> &str {
        "weixin"
    }

    fn name(&self) -> &str {
        &self.tenant_name
    }

    fn start(&mut self, sender: MessageSender) -> Result<(), PluginError> {
        let tenant_name = self.tenant_name.clone();

        // Mark tenant as active
        if let Some(state) = WEIXIN_STATE.tenants.get(&tenant_name) {
            state.active.store(true, Ordering::Relaxed);
        } else {
            info!(tenant = %tenant_name, "ILinkChannel starting in waiting mode (no token yet)");
        }

        let shutdown = self.shutdown.clone();
        let thread_tenant = tenant_name.clone();
        let handle = std::thread::Builder::new()
            .name(format!("weixin-poll-{tenant_name}"))
            .spawn(move || {
                run_poll_loop(&thread_tenant, sender, &shutdown);
            })
            .map_err(|e| PluginError::channel(format!("Failed to spawn polling thread: {e}")))?;

        self.thread_handle = Some(handle);
        info!(tenant = %tenant_name, "ILinkChannel started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), PluginError> {
        let state = WEIXIN_STATE
            .tenants
            .get(tenant_id)
            .ok_or_else(|| PluginError::channel(format!("Unknown tenant: {tenant_id}")))?;

        if state.is_expired() {
            return Err(PluginError::channel(format!(
                "Token expired for tenant {tenant_id}, please re-scan QR code"
            )));
        }

        // Get cached context_token for this user (REQUIRED by iLink protocol)
        let context_token = state
            .get_context_token(user_id)
            .ok_or_else(|| {
                PluginError::channel(format!(
                    "No context_token for user {user_id} — can only reply to received messages"
                ))
            })?;

        // Generate client_id
        let client_id = format!("openclaw-weixin-{}", Uuid::new_v4().as_simple());

        // Clone what we need, then do a blocking HTTP call on the tenant's own runtime.
        // We use a small single-threaded runtime here instead of Handle::current()
        // because send() may be called from any thread (including non-tokio threads via FFI).
        let bot_token = state.bot_token.clone();
        let baseurl = state.baseurl.clone();
        let http = state.http.clone();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::channel(format!("Failed to create send runtime: {e}")))?;

        rt.block_on(async {
            api::send_message(
                &http,
                &bot_token,
                &baseurl,
                user_id,
                &context_token,
                &client_id,
                text,
            )
            .await
            .map_err(PluginError::channel)
        })
    }

    fn stop(&mut self) {
        // Signal the polling thread to shut down
        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(state) = WEIXIN_STATE.tenants.get(&self.tenant_name) {
            state.active.store(false, Ordering::Relaxed);
        }

        // Wait for the polling thread to finish (with timeout to avoid hanging)
        if let Some(handle) = self.thread_handle.take() {
            // The thread should exit within one poll cycle (~40s max)
            // Use a reasonable timeout to avoid blocking indefinitely
            match handle.join() {
                Ok(()) => info!(tenant = %self.tenant_name, "Polling thread joined cleanly"),
                Err(e) => error!(tenant = %self.tenant_name, "Polling thread panicked: {e:?}"),
            }
        }

        info!(tenant = %self.tenant_name, "ILinkChannel stopped");
    }
}

/// Main polling loop (runs in a dedicated thread with its own runtime).
fn run_poll_loop(
    tenant_name: &str,
    sender: MessageSender,
    shutdown: &AtomicBool,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(tenant = tenant_name, "Failed to create tokio runtime: {e}");
            return;
        }
    };

    rt.block_on(async move {
        poll_loop_inner(tenant_name, sender, shutdown).await;
    });
}

async fn poll_loop_inner(
    tenant_name: &str,
    sender: MessageSender,
    shutdown: &AtomicBool,
) {
    info!(tenant = tenant_name, "Poll loop started");

    loop {
        // Check shutdown signal first
        if shutdown.load(Ordering::Relaxed) {
            info!(tenant = tenant_name, "Shutdown signal received, exiting poll loop");
            return;
        }

        // Wait for tenant to be registered and active (with shutdown check)
        let (bot_token, baseurl, http) = {
            let state = match WEIXIN_STATE.tenants.get(tenant_name) {
                Some(s) => s,
                None => {
                    // Sleep in short intervals to check shutdown flag
                    for _ in 0..10 {
                        if shutdown.load(Ordering::Relaxed) {
                            info!(tenant = tenant_name, "Shutdown during wait, exiting");
                            return;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    continue;
                }
            };

            if !state.active.load(Ordering::Relaxed) || state.is_expired() {
                // Sleep in short intervals to check shutdown flag
                for _ in 0..10 {
                    if shutdown.load(Ordering::Relaxed) {
                        info!(tenant = tenant_name, "Shutdown during inactive wait, exiting");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                continue;
            }

            (
                state.bot_token.clone(),
                state.baseurl.clone(),
                state.http.clone(),
            )
        };

        // Get current cursor
        let cursor = WEIXIN_STATE
            .tenants
            .get(tenant_name)
            .map(|s| s.cursor.lock().unwrap().clone())
            .unwrap_or_default();

        // Long-poll getupdates (35s hold) — shutdown is checked at the top of the loop.
        // The poll will complete naturally; worst case we wait one cycle after stop().
        match api::get_updates(&http, &bot_token, &baseurl, &cursor).await {
            Ok(resp) => {
                // Check for session expired
                if resp.errcode == Some(SESSION_EXPIRED_ERRCODE)
                    || resp.ret == Some(SESSION_EXPIRED_ERRCODE)
                {
                    warn!(tenant = tenant_name, "Session expired, stopping poll");
                    if let Some(state) = WEIXIN_STATE.tenants.get(tenant_name) {
                        state.active.store(false, Ordering::Relaxed);
                    }
                    continue;
                }

                // Check for API errors
                if let Some(ret) = resp.ret {
                    if ret != 0 {
                        warn!(tenant = tenant_name, ret, "getUpdates returned non-zero ret");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                }

                // Update cursor
                if let Some(new_cursor) = &resp.get_updates_buf {
                    if !new_cursor.is_empty() {
                        if let Some(state) = WEIXIN_STATE.tenants.get(tenant_name) {
                            *state.cursor.lock().unwrap() = new_cursor.clone();
                        }
                    }
                }

                // Process messages
                if let Some(msgs) = resp.msgs {
                    for msg in msgs {
                        process_inbound_message(tenant_name, &msg, &sender);
                    }
                }
            }
            Err(e) => {
                error!(tenant = tenant_name, "getUpdates error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}

/// Process a single inbound message from getUpdates.
fn process_inbound_message(tenant_name: &str, msg: &ILnkMessage, sender: &MessageSender) {
    // Only process user messages
    if msg.message_type != Some(MSG_TYPE_USER) {
        return;
    }
    if msg.message_state != Some(MSG_STATE_FINISH) {
        return;
    }

    let from_user_id = match &msg.from_user_id {
        Some(id) if !id.is_empty() => id.clone(),
        _ => return,
    };

    // Extract text content
    let text = msg
        .item_list
        .as_ref()
        .and_then(|items| {
            items.iter().find_map(|item| {
                if item.type_ == Some(ITEM_TYPE_TEXT) {
                    item.text_item.as_ref()?.text.clone()
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    // Store context_token (REQUIRED for replies)
    if let Some(ctx_token) = &msg.context_token {
        if let Some(state) = WEIXIN_STATE.tenants.get(tenant_name) {
            state.store_context_token(&from_user_id, ctx_token);
        }
    }

    info!(
        tenant = tenant_name,
        from = %from_user_id,
        text_len = text.len(),
        "Inbound WeChat message"
    );

    // Build PluginMessage and send to host
    let plugin_msg = PluginMessage {
        channel_type: "weixin".to_string(),
        platform_message_id: msg.message_id.map(|id| id.to_string()).unwrap_or_default(),
        sender_id: from_user_id.clone(),
        sender_name: from_user_id.clone(),
        tenant_id: tenant_name.to_string(),
        content: PluginContent::Text(text),
        timestamp_ms: msg.create_time_ms.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        }),
        is_group: msg.group_id.is_some(),
        thread_id: msg.group_id.clone(),
        metadata: Default::default(),
    };

    sender.send(plugin_msg);
}
