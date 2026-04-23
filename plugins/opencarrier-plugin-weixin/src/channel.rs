//! ILinkChannel — long-polling ChannelAdapter for WeChat personal account.
//!
//! Spawns a dedicated OS thread with its own tokio runtime (cdylib plugins
//! cannot safely use the host runtime). Polls `getupdates` (35s hold) and
//! dispatches inbound messages through the plugin SDK's MessageSender.

use crate::api;
use crate::token::WEIXIN_STATE;
use crate::types::*;
use opencarrier_plugin_sdk::{ChannelAdapter, MessageSender, PluginContent, PluginError, PluginMessage};
use std::sync::atomic::Ordering;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Channel adapter for a single iLink tenant (one scanned WeChat account).
pub struct ILinkChannel {
    tenant_name: String,
}

impl ILinkChannel {
    pub fn new(tenant_name: String) -> Self {
        Self { tenant_name }
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
            // Tenant not loaded yet — will be picked up when QR scan completes
            info!(tenant = %tenant_name, "ILinkChannel starting in waiting mode (no token yet)");
        }

        let thread_tenant = tenant_name.clone();
        std::thread::Builder::new()
            .name(format!("weixin-poll-{tenant_name}"))
            .spawn(move || {
                run_poll_loop(&thread_tenant, sender);
            })
            .map_err(|e| PluginError::channel(format!("Failed to spawn polling thread: {e}")))?;

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

        // Send via iLink API (block_on from sync context)
        let bot_token = state.bot_token.clone();
        let baseurl = state.baseurl.clone();
        let http = state.http.clone();

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(async {
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
        })
    }

    fn stop(&mut self) {
        if let Some(state) = WEIXIN_STATE.tenants.get(&self.tenant_name) {
            state.active.store(false, Ordering::Relaxed);
        }
        info!(tenant = %self.tenant_name, "ILinkChannel stopped");
    }
}

/// Main polling loop (runs in a dedicated thread with its own runtime).
fn run_poll_loop(tenant_name: &str, sender: MessageSender) {
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
        poll_loop_inner(tenant_name, sender).await;
    });
}

async fn poll_loop_inner(tenant_name: &str, sender: MessageSender) {
    info!(tenant = tenant_name, "Poll loop started");

    loop {
        // Wait for tenant to be registered and active
        let (bot_token, baseurl, http) = {
            let state = match WEIXIN_STATE.tenants.get(tenant_name) {
                Some(s) => s,
                None => {
                    // Tenant not registered yet, wait and retry
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            if !state.active.load(Ordering::Relaxed) || state.is_expired() {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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

        // Long-poll getupdates
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
