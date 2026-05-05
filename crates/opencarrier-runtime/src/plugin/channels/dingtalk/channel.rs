//! Per-tenant DingTalk channel adapter.
//!
//! Spawns an OS thread with a tokio runtime running the DingTalk WS client.

use crate::plugin::channels::dingtalk::api;
use crate::plugin::channels::dingtalk::token::AccessTokenCache;
use crate::plugin::channels::dingtalk::ws::DingTalkWsClient;
use crate::plugin::BuiltinChannel;
use opencarrier_types::plugin::PluginMessage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct DingTalkChannel {
    tenant_name: String,
    bot_uuid: String,
    token_cache: Arc<AccessTokenCache>,
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl DingTalkChannel {
    pub fn new(tenant_name: String, bot_uuid: String, token_cache: Arc<AccessTokenCache>) -> Self {
        Self {
            tenant_name,
            bot_uuid,
            token_cache,
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl BuiltinChannel for DingTalkChannel {
    fn channel_type(&self) -> &str {
        "dingtalk"
    }

    fn name(&self) -> &str {
        &self.tenant_name
    }

    fn tenant_id(&self) -> &str {
        &self.bot_uuid
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let tenant_name = self.tenant_name.clone();
        let bot_uuid = self.bot_uuid.clone();
        let token_cache = self.token_cache.clone();
        let shutdown = self.shutdown.clone();

        let handle = std::thread::Builder::new()
            .name(format!("dingtalk-ws-{tenant_name}"))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        warn!(tenant = %tenant_name, "Failed to create tokio runtime: {e}");
                        return;
                    }
                };

                let client =
                    DingTalkWsClient::new(tenant_name.clone(), bot_uuid, token_cache, shutdown);
                rt.block_on(client.run(&sender));
                info!(tenant = %tenant_name, "DingTalk WS client exited");
            })
            .map_err(|e| format!("Failed to spawn DingTalk channel thread: {e}"))?;

        self.thread_handle = Some(handle);
        Ok(())
    }

    fn send(&self, _tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let token_cache = self.token_cache.clone();
        let user_id = user_id.to_string();
        let text = text.to_string();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!("Runtime creation failed: {e}")));
                    return;
                }
            };

            let result = rt.block_on(async {
                let token = token_cache
                    .get_token()
                    .await
                    .map_err(|e| format!("Token error: {e}"))?;
                let http = token_cache.http().clone();
                let robot_code = token_cache.app_key().to_string();

                // Try direct message
                api::send_direct_message(&http, &token, &robot_code, &user_id, &text).await
            });

            let _ = tx.send(result);
        });

        rx.recv().map_err(|e| format!("Send thread disconnected: {e}"))?
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(()) => info!(tenant = %self.tenant_name, "DingTalk channel thread joined"),
                Err(e) => {
                    if let Some(s) = e.downcast_ref::<&str>() {
                        warn!(tenant = %self.tenant_name, "DingTalk channel thread panicked: {s}");
                    }
                }
            }
        }
    }
}
