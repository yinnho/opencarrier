//! FeishuChannel — WebSocket-based BuiltinChannel for Feishu/Lark.
//!
//! Spawns a dedicated OS thread with its own tokio runtime.
//! Connects to the Feishu WebSocket long-connection endpoint and dispatches
//! inbound messages through the host's native `mpsc::Sender<PluginMessage>`.

use crate::plugin::BuiltinChannel;
use crate::plugin::channels::feishu::token::TenantTokenCache;
use crate::plugin::channels::feishu::ws::FeishuWsClient;
use opencarrier_types::plugin::PluginMessage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

/// Channel adapter for a single Feishu tenant (one app_id).
pub struct FeishuChannel {
    tenant_name: String,
    token_cache: Arc<TenantTokenCache>,
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl FeishuChannel {
    pub fn new(tenant_name: String, token_cache: Arc<TenantTokenCache>) -> Self {
        Self {
            tenant_name,
            token_cache,
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl BuiltinChannel for FeishuChannel {
    fn channel_type(&self) -> &str {
        "feishu"
    }

    fn name(&self) -> &str {
        &self.tenant_name
    }

    fn tenant_id(&self) -> &str {
        &self.tenant_name
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let tenant_name = self.tenant_name.clone();
        let token_cache = self.token_cache.clone();
        let shutdown = self.shutdown.clone();
        let thread_tenant = tenant_name.clone();

        let handle = std::thread::Builder::new()
            .name(format!("feishu-ws-{tenant_name}"))
            .spawn(move || {
                run_ws_loop(&thread_tenant, token_cache, shutdown, sender);
            })
            .map_err(|e| format!("Failed to spawn Feishu WS thread: {e}"))?;

        self.thread_handle = Some(handle);
        info!(tenant = %tenant_name, "FeishuChannel started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        // Verify tenant matches
        if tenant_id != self.tenant_name {
            return Err(format!(
                "Tenant mismatch: expected {}, got {}",
                self.tenant_name, tenant_id
            ));
        }

        let token = self
            .token_cache
            .get_token()
            .map_err(|e| format!("Token error: {e}"))?;

        let content = serde_json::json!({ "text": text }).to_string();
        let http = self.token_cache.http();
        let base = self.token_cache.api_base().to_string();

        // Build a temporary runtime for the async HTTP call
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create send runtime: {e}"))?;

        rt.block_on(async {
            let resp = crate::plugin::channels::feishu::api::send_message(
                http, &token, &base, user_id, "open_id", "text", &content,
            )
            .await?;

            if resp.code != 0 {
                return Err(format!(
                    "Feishu send error: code={} msg={}",
                    resp.code, resp.msg
                ));
            }
            Ok(())
        })
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(()) => info!(tenant = %self.tenant_name, "WS thread joined cleanly"),
                Err(e) => {
                    if let Some(s) = e.downcast_ref::<&str>() {
                        tracing::error!(tenant = %self.tenant_name, "WS thread panicked: {s}");
                    }
                }
            }
        }

        info!(tenant = %self.tenant_name, "FeishuChannel stopped");
    }
}

/// Main WebSocket loop (runs in a dedicated thread with its own runtime).
fn run_ws_loop(
    tenant_name: &str,
    token_cache: Arc<TenantTokenCache>,
    shutdown: Arc<AtomicBool>,
    sender: mpsc::Sender<PluginMessage>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(tenant = tenant_name, "Failed to create tokio runtime: {e}");
            return;
        }
    };

    let ws_client = FeishuWsClient::new(
        tenant_name.to_string(),
        token_cache,
        shutdown,
    );

    rt.block_on(async move {
        ws_client.run(&sender).await;
    });
}
