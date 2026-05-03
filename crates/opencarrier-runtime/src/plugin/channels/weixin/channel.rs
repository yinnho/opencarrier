//! ILinkChannel — long-polling BuiltinChannel for WeChat personal account.
//!
//! Spawns a dedicated OS thread with its own tokio runtime.
//! Polls `getupdates` (35s hold) and dispatches inbound messages through
//! the host's native `mpsc::Sender<PluginMessage>`.

use crate::plugin::channels::weixin::api;
use crate::plugin::channels::weixin::token::WEIXIN_STATE;
use crate::plugin::channels::weixin::types::*;
use opencarrier_types::plugin::{PluginContent, PluginMessage};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::plugin::BuiltinChannel;

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

impl BuiltinChannel for ILinkChannel {
    fn channel_type(&self) -> &str {
        "weixin"
    }

    fn name(&self) -> &str {
        &self.tenant_name
    }

    fn tenant_id(&self) -> &str {
        &self.tenant_name
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
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
            .map_err(|e| format!("Failed to spawn polling thread: {e}"))?;

        self.thread_handle = Some(handle);
        info!(tenant = %tenant_name, "ILinkChannel started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let state = WEIXIN_STATE
            .tenants
            .get(tenant_id)
            .ok_or_else(|| format!("Unknown tenant: {tenant_id}"))?;

        if state.is_expired() {
            return Err(format!(
                "Token expired for tenant {tenant_id}, please re-scan QR code"
            ));
        }

        // Get cached context_token for this user (REQUIRED by iLink protocol)
        let context_token = state
            .get_context_token(user_id)
            .ok_or_else(|| {
                format!(
                    "No context_token for user {user_id} — can only reply to received messages"
                )
            })?;

        // Generate client_id
        let client_id = format!("openclaw-weixin-{}", Uuid::new_v4().as_simple());

        let bot_token = state.bot_token.clone();
        let baseurl = state.baseurl.clone();
        let http = state.http.clone();

        let user_id = user_id.to_string();
        let context_token = context_token.to_string();
        let text = text.to_string();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!("Failed to create send runtime: {e}")));
                    return;
                }
            };
            let result = rt.block_on(async {
                api::send_message(
                    &http,
                    &bot_token,
                    &baseurl,
                    &user_id,
                    &context_token,
                    &client_id,
                    &text,
                )
                .await
            });
            let _ = tx.send(result);
        });

        rx.recv().map_err(|e| format!("Send thread disconnected: {e}"))?
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(state) = WEIXIN_STATE.tenants.get(&self.tenant_name) {
            state.active.store(false, Ordering::Relaxed);
        }

        if let Some(handle) = self.thread_handle.take() {
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
    sender: mpsc::Sender<PluginMessage>,
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
    sender: mpsc::Sender<PluginMessage>,
    shutdown: &AtomicBool,
) {
    info!(tenant = tenant_name, "Poll loop started");

    loop {
        if shutdown.load(Ordering::Relaxed) {
            info!(tenant = tenant_name, "Shutdown signal received, exiting poll loop");
            return;
        }

        let (bot_token, baseurl, http) = {
            let state = match WEIXIN_STATE.tenants.get(tenant_name) {
                Some(s) => s,
                None => {
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

        let cursor = WEIXIN_STATE
            .tenants
            .get(tenant_name)
            .map(|s| s.cursor.lock().unwrap().clone())
            .unwrap_or_default();

        match api::get_updates(&http, &bot_token, &baseurl, &cursor).await {
            Ok(resp) => {
                if resp.errcode == Some(SESSION_EXPIRED_ERRCODE)
                    || resp.ret == Some(SESSION_EXPIRED_ERRCODE)
                {
                    warn!(tenant = tenant_name, "Session expired, stopping poll");
                    if let Some(state) = WEIXIN_STATE.tenants.get(tenant_name) {
                        state.active.store(false, Ordering::Relaxed);
                    }
                    continue;
                }

                if let Some(ret) = resp.ret {
                    if ret != 0 {
                        warn!(tenant = tenant_name, ret, "getUpdates returned non-zero ret");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                }

                if let Some(new_cursor) = &resp.get_updates_buf {
                    if !new_cursor.is_empty() {
                        if let Some(state) = WEIXIN_STATE.tenants.get(tenant_name) {
                            *state.cursor.lock().unwrap() = new_cursor.clone();
                        }
                    }
                }

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

fn process_inbound_message(
    tenant_name: &str,
    msg: &ILnkMessage,
    sender: &mpsc::Sender<PluginMessage>,
) {
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

    if let Err(e) = sender.try_send(plugin_msg) {
        warn!(error = %e, "Plugin message channel full, dropping message");
    }
}

// ---------------------------------------------------------------------------
// TenantWatcher — monitors for new tenants added after plugin startup
// ---------------------------------------------------------------------------

/// Dynamic tenant watcher that polls `WEIXIN_STATE` for new tenants and
/// starts polling threads for them. Handles outbound `send()` for any tenant.
pub struct TenantWatcher {
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl Default for TenantWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl TenantWatcher {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl BuiltinChannel for TenantWatcher {
    fn channel_type(&self) -> &str {
        "weixin"
    }

    fn name(&self) -> &str {
        "__watcher__"
    }

    fn tenant_id(&self) -> &str {
        ""
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let shutdown = self.shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("weixin-tenant-watcher".to_string())
            .spawn(move || {
                watcher_loop(sender, shutdown);
            })
            .map_err(|e| format!("Failed to spawn watcher thread: {e}"))?;
        self.thread_handle = Some(handle);
        info!("WeChat TenantWatcher started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let state = WEIXIN_STATE
            .tenants
            .get(tenant_id)
            .ok_or_else(|| format!("Unknown tenant: {tenant_id}"))?;

        if state.is_expired() {
            return Err(format!("Token expired for tenant {tenant_id}"));
        }

        let context_token = state.get_context_token(user_id).ok_or_else(|| {
            format!(
                "No context_token for user {user_id} — can only reply to received messages"
            )
        })?;

        let client_id = format!("openclaw-weixin-{}", Uuid::new_v4().as_simple());
        let bot_token = state.bot_token.clone();
        let baseurl = state.baseurl.clone();
        let http = state.http.clone();
        let user_id = user_id.to_string();
        let context_token = context_token.to_string();
        let text = text.to_string();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!("Failed to create send runtime: {e}")));
                    return;
                }
            };
            let result = rt.block_on(async {
                api::send_message(
                    &http,
                    &bot_token,
                    &baseurl,
                    &user_id,
                    &context_token,
                    &client_id,
                    &text,
                )
                .await
            });
            let _ = tx.send(result);
        });

        rx.recv().map_err(|e| format!("Send thread disconnected: {e}"))?
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(()) => info!("TenantWatcher thread joined cleanly"),
                Err(e) => error!("TenantWatcher thread panicked: {e:?}"),
            }
        }
        info!("TenantWatcher stopped");
    }
}

fn watcher_loop(sender: mpsc::Sender<PluginMessage>, shutdown: Arc<AtomicBool>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("Failed to create watcher tokio runtime: {e}");
            return;
        }
    };

    rt.block_on(async move {
        let mut spawned: HashSet<String> = HashSet::new();

        loop {
            if shutdown.load(Ordering::Relaxed) {
                info!("TenantWatcher shutdown signal received");
                return;
            }

            WEIXIN_STATE.load_new_from_dir();

            for entry in WEIXIN_STATE.tenants.iter() {
                let name = entry.key().clone();
                if spawned.contains(&name) {
                    continue;
                }
                let state = entry.value();
                if state.is_expired() {
                    continue;
                }
                if state.active.load(Ordering::Relaxed) {
                    spawned.insert(name);
                    continue;
                }
                state.active.store(true, Ordering::Relaxed);
                spawned.insert(name.clone());
                let s = sender.clone();
                let sh = shutdown.clone();
                let thread_name = name.clone();
                let poll_name = name.clone();
                info!(tenant = %name, "TenantWatcher spawning poll thread for new tenant");
                if let Err(e) = std::thread::Builder::new()
                    .name(format!("weixin-dyn-{thread_name}"))
                    .spawn(move || {
                        run_poll_loop(&poll_name, s, &sh);
                    })
                {
                    error!(tenant = %name, "Failed to spawn poll thread: {e}");
                }
            }

            for _ in 0..10 {
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    });
}
