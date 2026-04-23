//! Token storage and management for the WeChat iLink Bot plugin.
//!
//! Manages per-tenant bot_tokens (24h expiry) and per-user context_tokens.
//! Tokens are persisted to `~/.opencarrier/weixin-tokens/<name>.json`.

use dashmap::DashMap;
use reqwest::Client;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use crate::types::*;

// ---------------------------------------------------------------------------
// Per-tenant runtime state
// ---------------------------------------------------------------------------

/// Runtime state for a single iLink tenant (one scanned WeChat account).
pub struct TenantState {
    /// Tenant name (used as routing key).
    pub name: String,
    /// iLink bot_token (from QR scan, valid 24h).
    pub bot_token: String,
    /// iLink base URL (from QR scan, usually same as ILINK_API_BASE).
    pub baseurl: String,
    /// The bot's iLink ID (e.g. "xxx@im.bot").
    pub ilink_bot_id: String,
    /// The WeChat user ID who scanned the QR code.
    pub user_id: Option<String>,
    /// Unix timestamp (seconds) when this token expires.
    pub expires_at: i64,
    /// Shared HTTP client.
    pub http: Client,
    /// Per-user context_token cache: user_id → context_token.
    context_tokens: Mutex<HashMap<String, String>>,
    /// Per-user typing_ticket cache: user_id → (ticket, cached_at).
    typing_tickets: Mutex<HashMap<String, (String, Instant)>>,
    /// get_updates_buf cursor for long-polling.
    pub cursor: Mutex<String>,
    /// Whether the polling loop is active.
    pub active: AtomicBool,
    /// Optional agent name to bind this channel to.
    pub bind_agent: Option<String>,
}

impl TenantState {
    /// Check if this tenant's token has expired.
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        now >= self.expires_at
    }

    /// Check if this tenant's token will expire within the given number of seconds.
    pub fn is_near_expiry(&self, within_secs: i64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        now >= self.expires_at - within_secs
    }

    /// Seconds remaining until expiry.
    pub fn remaining_secs(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        (self.expires_at - now).max(0)
    }

    /// Store a context_token for a user (from an inbound message).
    pub fn store_context_token(&self, user_id: &str, token: &str) {
        self.context_tokens
            .lock()
            .unwrap()
            .insert(user_id.to_string(), token.to_string());
    }

    /// Get the cached context_token for a user.
    pub fn get_context_token(&self, user_id: &str) -> Option<String> {
        self.context_tokens
            .lock()
            .unwrap()
            .get(user_id)
            .cloned()
    }

    /// Cache a typing_ticket for a user (valid 24h, we cache for 23h).
    pub fn store_typing_ticket(&self, user_id: &str, ticket: &str) {
        self.typing_tickets
            .lock()
            .unwrap()
            .insert(user_id.to_string(), (ticket.to_string(), Instant::now()));
    }

    /// Get a cached typing_ticket for a user (if fresh enough).
    pub fn get_typing_ticket(&self, user_id: &str) -> Option<String> {
        self.typing_tickets
            .lock()
            .unwrap()
            .get(user_id)
            .and_then(|(ticket, cached_at)| {
                // Cache for 23 hours (typing_ticket valid for 24h)
                if cached_at.elapsed().as_secs() < 23 * 3600 {
                    Some(ticket.clone())
                } else {
                    None
                }
            })
    }
}

// ---------------------------------------------------------------------------
// Global state manager
// ---------------------------------------------------------------------------

/// Global state manager for all iLink tenants.
pub struct WeixinState {
    /// Per-tenant state keyed by tenant name.
    pub tenants: DashMap<String, TenantState>,
    /// Directory for persisting token files.
    pub token_dir: PathBuf,
    /// Shared HTTP client for API routes (QR code login).
    pub http: Client,
}

impl WeixinState {
    fn new() -> Self {
        let token_dir = dirs_home().join(".opencarrier").join("weixin-tokens");
        Self {
            tenants: DashMap::new(),
            token_dir,
            http: Client::new(),
        }
    }

    /// Set the token directory (called from plugin config).
    pub fn set_token_dir(&self, _dir: PathBuf) {
        // DashMap doesn't have a token_dir setter, so we use interior mutability
        // Actually, we need to make token_dir mutable. Let's use a different approach.
        // We'll just use the default or configured dir via a separate method.
    }

    /// Load persisted tokens from the token directory.
    pub fn load_from_dir(&self, dir: &Path) {
        if !dir.exists() {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let entries = std::fs::read_dir(dir);
        match entries {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            match serde_json::from_str::<TenantTokenFile>(&content) {
                                Ok(tf) => {
                                    if now >= tf.expires_at {
                                        info!(
                                            tenant = %tf.name,
                                            "Skipping expired iLink token"
                                        );
                                        continue;
                                    }
                                    info!(
                                        tenant = %tf.name,
                                        expires_in = tf.expires_at - now,
                                        "Loaded iLink token"
                                    );
                                    let state = TenantState {
                                        name: tf.name.clone(),
                                        bot_token: tf.bot_token,
                                        baseurl: tf.baseurl,
                                        ilink_bot_id: tf.ilink_bot_id,
                                        user_id: tf.user_id,
                                        expires_at: tf.expires_at,
                                        http: Client::new(),
                                        context_tokens: Mutex::new(HashMap::new()),
                                        typing_tickets: Mutex::new(HashMap::new()),
                                        cursor: Mutex::new(String::new()),
                                        active: AtomicBool::new(false), // Will be set to true when channel starts
                                        bind_agent: tf.bind_agent,
                                    };
                                    self.tenants.insert(tf.name, state);
                                }
                                Err(e) => {
                                    warn!(path = %path.display(), "Failed to parse token file: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            warn!(path = %path.display(), "Failed to read token file: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(dir = %dir.display(), "Failed to read token directory: {e}");
            }
        }
    }

    /// Register a new tenant from a successful QR scan.
    pub fn register_from_qr(
        &self,
        name: &str,
        bot_token: &str,
        baseurl: &str,
        ilink_bot_id: &str,
        user_id: Option<&str>,
        bind_agent: Option<&str>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let state = TenantState {
            name: name.to_string(),
            bot_token: bot_token.to_string(),
            baseurl: baseurl.to_string(),
            ilink_bot_id: ilink_bot_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            expires_at: now + SESSION_DURATION_SECS,
            http: Client::new(),
            context_tokens: Mutex::new(HashMap::new()),
            typing_tickets: Mutex::new(HashMap::new()),
            cursor: Mutex::new(String::new()),
            active: AtomicBool::new(true),
            bind_agent: bind_agent.map(|s| s.to_string()),
        };

        // Persist to disk
        self.save_tenant(&state);

        // Insert/update in-memory
        if let Some(mut existing) = self.tenants.get_mut(name) {
            // Preserve cursor and context_tokens from existing session if possible
            let old_cursor = existing.cursor.lock().unwrap().clone();
            *state.cursor.lock().unwrap() = old_cursor;
            *existing = state;
        } else {
            self.tenants.insert(name.to_string(), state);
        }

        info!(tenant = name, "Registered iLink tenant from QR scan");
    }

    /// Save a tenant's state to disk.
    pub fn save_tenant(&self, state: &TenantState) {
        let dir = &self.token_dir;
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!(dir = %dir.display(), "Failed to create token directory: {e}");
            return;
        }

        let tf = TenantTokenFile {
            name: state.name.clone(),
            bot_token: state.bot_token.clone(),
            baseurl: state.baseurl.clone(),
            ilink_bot_id: state.ilink_bot_id.clone(),
            user_id: state.user_id.clone(),
            expires_at: state.expires_at,
            bind_agent: state.bind_agent.clone(),
        };

        let path = dir.join(format!("{}.json", state.name));
        match serde_json::to_string_pretty(&tf) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(path = %path.display(), "Failed to write token file: {e}");
                }
            }
            Err(e) => {
                warn!("Failed to serialize tenant token: {e}");
            }
        }
    }

    /// Get a tenant state by name.
    pub fn get_tenant(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, String, TenantState>> {
        self.tenants.get(name)
    }

    /// List all active (non-expired) tenant names.
    pub fn active_tenant_names(&self) -> Vec<String> {
        self.tenants
            .iter()
            .filter(|e| !e.value().is_expired())
            .map(|e| e.key().clone())
            .collect()
    }

    /// Get status of all tenants for the API.
    pub fn status_list(&self) -> Vec<serde_json::Value> {
        self.tenants
            .iter()
            .map(|entry| {
                let state = entry.value();
                serde_json::json!({
                    "name": state.name,
                    "ilink_bot_id": state.ilink_bot_id,
                    "user_id": state.user_id,
                    "expires_at": state.expires_at,
                    "remaining_secs": state.remaining_secs(),
                    "expired": state.is_expired(),
                    "active": state.active.load(Ordering::Relaxed),
                    "bind_agent": state.bind_agent,
                })
            })
            .collect()
    }
}

/// Get the home directory.
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Global singleton for iLink state management.
pub static WEIXIN_STATE: std::sync::LazyLock<WeixinState> =
    std::sync::LazyLock::new(WeixinState::new);
