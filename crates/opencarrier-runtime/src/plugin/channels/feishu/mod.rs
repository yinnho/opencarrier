//! Feishu/Lark built-in channel adapter and tools.
//!
//! Provides:
//! - `FeishuWatcher` — scans bot.toml and spawns WebSocket connections
//! - `FeishuChannel` — per-tenant WebSocket message receiver
//! - 80+ Feishu Open API tools via `BuiltinPluginRegistry`

pub mod api;
pub mod api_ext;
pub mod channel;
pub mod pbbp2;
pub mod token;
pub mod tools;
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

/// Runtime entry stored in FEISHU_TENANTS — config + pre-built token cache.
pub(crate) struct FeishuTenantEntry {
    pub config: types::FeishuTenantConfig,
    pub token_cache: Arc<token::TenantTokenCache>,
}

impl FeishuTenantEntry {
    pub fn new(config: types::FeishuTenantConfig) -> Self {
        let api_base = config.api_base().to_string();
        let token_cache = Arc::new(token::TenantTokenCache::new(
            config.app_id.clone(),
            config.app_secret.clone(),
            &api_base,
        ));
        Self { config, token_cache }
    }
}

/// Global registry of all configured Feishu tenants.
pub(crate) static FEISHU_TENANTS: std::sync::LazyLock<DashMap<String, FeishuTenantEntry>> =
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

    for dir_name in ["feishu", "opencarrier-plugin-feishu"] {
        let dir = home.join("plugins").join(dir_name);
        if dir.exists() {
            return Some(dir);
        }
    }

    Some(home.join("plugins").join("feishu"))
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

fn load_bot_config(bot_config: &serde_json::Value) -> Option<FeishuTenantEntry> {
    let bot_uuid = bot_config["_bot_id"].as_str().unwrap_or("").to_string();
    let name = bot_config["name"].as_str().unwrap_or("").to_string();

    if name.is_empty() || bot_uuid.is_empty() {
        tracing::warn!("Skipping Feishu bot with empty name or bot_id");
        return None;
    }

    // Read secret: try env var first, fall back to inline config value
    let secret_env = bot_config["secret_env"].as_str().unwrap_or("FEISHU_APP_SECRET");
    let app_secret = match std::env::var(secret_env) {
        Ok(s) if !s.is_empty() => s,
        _ => {
            let inline = bot_config["app_secret"].as_str().unwrap_or("").to_string();
            if !inline.is_empty() {
                tracing::warn!(
                    bot = %name,
                    env_var = %secret_env,
                    "Using inline app_secret from config — consider setting env var instead"
                );
            }
            inline
        }
    };

    let app_id = bot_config["app_id"].as_str().unwrap_or("").to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        tracing::warn!(bot = %name, "Skipping Feishu bot: missing app_id or app_secret");
        return None;
    }

    let brand = bot_config["brand"].as_str().unwrap_or("feishu").to_string();

    let cfg = types::FeishuTenantConfig {
        name: name.clone(),
        bot_uuid: bot_uuid.clone(),
        app_id,
        app_secret,
        brand,
    };

    tracing::info!(bot = %name, bot_uuid = %bot_uuid, brand = %cfg.brand, "Registered Feishu bot");
    Some(FeishuTenantEntry::new(cfg))
}

// ---------------------------------------------------------------------------
// Feishu watcher loop — detects new bots at runtime and spawns channels
// ---------------------------------------------------------------------------

/// Background loop that monitors the Feishu plugin directory for new bots
/// added at runtime and spawns WebSocket channels for each new tenant.
fn feishu_watcher_loop(
    plugin_dir: &Path,
    sender: mpsc::Sender<PluginMessage>,
    shutdown: Arc<AtomicBool>,
) {
    let mut spawned: HashSet<String> = HashSet::new();

    // Spawn channels for bots that exist at startup
    if let Some(configs) = scan_and_spawn(plugin_dir, &sender) {
        for bot_id in configs {
            spawned.insert(bot_id);
        }
    }

    // Periodically check for new bots
    loop {
        if shutdown.load(Ordering::Relaxed) {
            info!("Feishu watcher shutdown signal received");
            return;
        }

        std::thread::sleep(Duration::from_secs(10));

        if shutdown.load(Ordering::Relaxed) {
            return;
        }

        let configs = scan_bot_configs(plugin_dir);
        for (bot_uuid, config) in configs {
            if spawned.contains(&bot_uuid) {
                continue;
            }

            let entry = match load_bot_config(&config) {
                Some(e) => e,
                None => continue,
            };
            let tenant_name = entry.config.name.clone();
            let token_cache = entry.token_cache.clone();
            FEISHU_TENANTS.insert(bot_uuid.clone(), entry);
            spawned.insert(bot_uuid.clone());

            let tx = sender.clone();
            let bu = bot_uuid.clone();
            std::thread::spawn(move || {
                let mut ch = channel::FeishuChannel::new(tenant_name.clone(), bu, token_cache);
                if let Err(e) = ch.start(tx) {
                    warn!(tenant = %tenant_name, "Feishu channel start error: {e}");
                }
            });
        }
    }
}

/// Scan plugin directory and spawn channels for all discovered bots.
/// Returns the set of tenant names that were spawned.
fn scan_and_spawn(
    plugin_dir: &Path,
    sender: &mpsc::Sender<PluginMessage>,
) -> Option<HashSet<String>> {
    let configs = scan_bot_configs(plugin_dir);
    if configs.is_empty() {
        info!("No Feishu bot configs found, watcher idle");
        return None;
    }

    let mut spawned = HashSet::new();
    for (bot_uuid, config) in configs {
        let entry = match load_bot_config(&config) {
            Some(e) => e,
            None => continue,
        };

        let tenant_name = entry.config.name.clone();
        let token_cache = entry.token_cache.clone();
        FEISHU_TENANTS.insert(bot_uuid.clone(), entry);

        let tx = sender.clone();
        let tn = tenant_name.clone();
        let bu = bot_uuid.clone();
        std::thread::spawn(move || {
            let mut ch = channel::FeishuChannel::new(tn.clone(), bu, token_cache);
            if let Err(e) = ch.start(tx) {
                warn!(tenant = %tn, "Feishu channel start error: {e}");
            }
        });

        spawned.insert(bot_uuid);
    }

    info!("Feishu watcher loop started, monitoring for new bots");
    Some(spawned)
}

// ---------------------------------------------------------------------------
// FeishuWatcher — watches for feishu bots and spawns WS connections
// ---------------------------------------------------------------------------

/// Watcher for Feishu/Lark bots.
///
/// Scans `plugins/feishu/bot/<uuid>/bot.toml` on start, loads tenants into
/// `FEISHU_TENANTS`, and spawns a `FeishuChannel` for each bot.
pub struct FeishuWatcher {
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl Default for FeishuWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuWatcher {
    pub fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }
}

impl BuiltinChannel for FeishuWatcher {
    fn channel_type(&self) -> &str {
        "feishu"
    }

    fn name(&self) -> &str {
        "Feishu Watcher"
    }

    fn tenant_id(&self) -> &str {
        ""
    }

    fn start(&mut self, sender: mpsc::Sender<PluginMessage>) -> Result<(), String> {
        let plugin_dir = find_plugin_dir()
            .ok_or_else(|| "Cannot find Feishu plugin directory".to_string())?;

        let shutdown = self.shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("feishu-watcher".to_string())
            .spawn(move || {
                feishu_watcher_loop(&plugin_dir, sender, shutdown);
            })
            .map_err(|e| format!("Failed to spawn Feishu watcher thread: {e}"))?;
        self.thread_handle = Some(handle);
        info!("Feishu watcher started");
        Ok(())
    }

    fn send(&self, tenant_id: &str, user_id: &str, text: &str) -> Result<(), String> {
        let entry = FEISHU_TENANTS
            .get(tenant_id)
            .ok_or_else(|| format!("Unknown tenant: {tenant_id}"))?;

        let content = serde_json::json!({ "text": text }).to_string();
        let token_cache = entry.token_cache.clone();
        let user_id = user_id.to_string();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!(
                        "Runtime creation failed: {e}"
                    )));
                    return;
                }
            };
            let result = rt.block_on(async {
                let token = token_cache
                    .get_token()
                    .await
                    .map_err(|e| format!("Token error: {e}"))?;
                let http = token_cache.http().clone();
                let base = token_cache.api_base().to_string();
                let resp = api::send_message(&http, &token, &base, &user_id, "open_id", "text", &content)
                    .await?;

                if resp.code != 0 {
                    return Err(format!(
                        "Feishu send error: code={} msg={}",
                        resp.code, resp.msg
                    ));
                }
                Ok(())
            });
            let _ = tx.send(result);
        });

        rx.recv().map_err(|e| format!("Send thread disconnected: {e}"))?
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(()) => info!("Feishu watcher thread joined"),
                Err(e) => {
                    if let Some(s) = e.downcast_ref::<&str>() {
                        warn!("Feishu watcher thread panicked: {s}");
                    }
                }
            }
        }
    }
}
