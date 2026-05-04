//! DingTalk built-in channel adapter.
//!
//! Provides:
//! - `DingTalkWatcher` — scans bot.toml and spawns WebSocket connections
//! - `DingTalkChannel` — per-tenant WebSocket message receiver

pub mod api;
pub mod channel;
pub mod token;
pub mod types;
pub mod ws;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use opencarrier_types::plugin::PluginMessage;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::plugin::BuiltinChannel;

// ---------------------------------------------------------------------------
// Global tenant registry
// ---------------------------------------------------------------------------

pub(crate) struct DingTalkTenantEntry {
    pub config: types::DingTalkTenantConfig,
    pub token_cache: Arc<token::AccessTokenCache>,
}

impl DingTalkTenantEntry {
    pub fn new(config: types::DingTalkTenantConfig) -> Self {
        let token_cache = Arc::new(token::AccessTokenCache::new(
            config.app_key.clone(),
            config.app_secret.clone(),
        ));
        Self { config, token_cache }
    }
}

pub(crate) static DINGTALK_TENANTS: std::sync::LazyLock<DashMap<String, DingTalkTenantEntry>> =
    std::sync::LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_plugin_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("OPENCARRIER_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".opencarrier"))
        })?;

    for dir_name in ["dingtalk", "opencarrier-plugin-dingtalk"] {
        let dir = home.join("plugins").join(dir_name);
        if dir.exists() {
            return Some(dir);
        }
    }

    Some(home.join("plugins").join("dingtalk"))
}

fn scan_bot_configs(plugin_dir: &Path) -> Vec<(String, serde_json::Value)> {
    let mut configs = Vec::new();
    let bot_dir = plugin_dir.join("bot");

    let entries = match std::fs::read_dir(&bot_dir) {
        Ok(e) => e,
        Err(_) => return configs,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let bot_uuid = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let bot_toml = path.join("bot.toml");
        if !bot_toml.exists() {
            continue;
        }
        match std::fs::read_to_string(&bot_toml) {
            Ok(content) => {
                if let Ok(mut val) = content.parse::<toml::Value>() {
                    if let Some(table) = val.as_table_mut() {
                        table.insert(
                            "_bot_id".to_string(),
                            toml::Value::String(bot_uuid.clone()),
                        );
                    }
                    if let Ok(json) = serde_json::to_value(val) {
                        configs.push((bot_uuid, json));
                    }
                }
            }
            Err(e) => warn!(path = %bot_toml.display(), "Failed to read bot.toml: {e}"),
        }
    }

    configs
}

fn load_bot_config(bot_config: &serde_json::Value) -> Option<DingTalkTenantEntry> {
    let bot_uuid = bot_config["_bot_id"].as_str().unwrap_or("").to_string();
    let name = bot_config["name"].as_str().unwrap_or("").to_string();

    if name.is_empty() || bot_uuid.is_empty() {
        warn!("Skipping DingTalk bot with empty name or bot_id");
        return None;
    }

    let secret_env = bot_config["secret_env"].as_str().unwrap_or("DINGTALK_APP_SECRET");
    let app_secret = match std::env::var(secret_env) {
        Ok(s) if !s.is_empty() => s,
        _ => {
            let inline = bot_config["app_secret"].as_str().unwrap_or("").to_string();
            if !inline.is_empty() {
                warn!(
                    bot = %name,
                    env_var = %secret_env,
                    "Using inline app_secret from config — consider setting env var instead"
                );
            }
            inline
        }
    };

    let app_key = bot_config["app_key"].as_str().unwrap_or("").to_string();
    if app_key.is_empty() || app_secret.is_empty() {
        warn!(bot = %name, "Skipping DingTalk bot: missing app_key or app_secret");
        return None;
    }

    let cfg = types::DingTalkTenantConfig {
        name: name.clone(),
        app_key,
        app_secret,
    };

    Some(DingTalkTenantEntry::new(cfg))
}

// ---------------------------------------------------------------------------
// Watcher loop
// ---------------------------------------------------------------------------

fn dingtalk_watcher_loop(
    plugin_dir: &Path,
    sender: mpsc::Sender<PluginMessage>,
    shutdown: Arc<AtomicBool>,
) {
    let mut spawned: HashSet<String> = HashSet::new();

    if let Some(configs) = scan_and_spawn(plugin_dir, &sender) {
        for name in configs {
            spawned.insert(name);
        }
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            info!("DingTalk watcher shutdown signal received");
            return;
        }

        std::thread::sleep(Duration::from_secs(10));

        if shutdown.load(Ordering::Relaxed) {
            return;
        }

        let configs = scan_bot_configs(plugin_dir);
        for (_uuid, config) in configs {
            let entry = match load_bot_config(&config) {
                Some(e) => e,
                None => continue,
            };
            let tenant_name = entry.config.name.clone();
            if spawned.contains(&tenant_name) {
                continue;
            }

            let token_cache = entry.token_cache.clone();
            DINGTALK_TENANTS.insert(tenant_name.clone(), entry);
            spawned.insert(tenant_name.clone());

            info!(tenant = %tenant_name, "New DingTalk bot discovered, spawning channel");

            let tx = sender.clone();
            std::thread::spawn(move || {
                let mut ch = channel::DingTalkChannel::new(tenant_name.clone(), token_cache);
                if let Err(e) = ch.start(tx) {
                    warn!(tenant = %tenant_name, "DingTalk channel start error: {e}");
                }
            });
        }
    }
}

fn scan_and_spawn(
    plugin_dir: &Path,
    sender: &mpsc::Sender<PluginMessage>,
) -> Option<HashSet<String>> {
    let configs = scan_bot_configs(plugin_dir);
    if configs.is_empty() {
        info!("No DingTalk bot configs found, watcher idle");
        return None;
    }

    let mut spawned = HashSet::new();
    for (_uuid, config) in configs {
        let entry = match load_bot_config(&config) {
            Some(e) => e,
            None => continue,
        };

        let tenant_name = entry.config.name.clone();
        let token_cache = entry.token_cache.clone();
        DINGTALK_TENANTS.insert(tenant_name.clone(), entry);

        let tx = sender.clone();
        let tn = tenant_name.clone();
        std::thread::spawn(move || {
            let mut ch = channel::DingTalkChannel::new(tn.clone(), token_cache);
            if let Err(e) = ch.start(tx) {
                warn!(tenant = %tn, "DingTalk channel start error: {e}");
            }
        });

        spawned.insert(tenant_name);
    }

    info!("DingTalk watcher loop started, monitoring for new bots");
    Some(spawned)
}

// ---------------------------------------------------------------------------
// DingTalkWatcher
// ---------------------------------------------------------------------------

pub struct DingTalkWatcher {
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl Default for DingTalkWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl DingTalkWatcher {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl BuiltinChannel for DingTalkWatcher {
    fn channel_type(&self) -> &str {
        "dingtalk"
    }

    fn name(&self) -> &str {
        "DingTalk Watcher"
    }

    fn tenant_id(&self) -> &str {
        ""
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let plugin_dir = find_plugin_dir()
            .ok_or_else(|| "Cannot find DingTalk plugin directory".to_string())?;

        let shutdown = self.shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("dingtalk-watcher".to_string())
            .spawn(move || {
                dingtalk_watcher_loop(&plugin_dir, sender, shutdown);
            })
            .map_err(|e| format!("Failed to spawn DingTalk watcher thread: {e}"))?;
        self.thread_handle = Some(handle);
        info!("DingTalk watcher started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let entry = DINGTALK_TENANTS
            .get(tenant_id)
            .ok_or_else(|| format!("Unknown DingTalk tenant: {tenant_id}"))?;

        let token_cache = entry.token_cache.clone();
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
                Ok(()) => info!("DingTalk watcher thread joined"),
                Err(e) => {
                    if let Some(s) = e.downcast_ref::<&str>() {
                        warn!("DingTalk watcher thread panicked: {s}");
                    }
                }
            }
        }
    }
}
