//! MCP config fetch and per-tenant caching.

use dashmap::DashMap;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use super::auth;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct McpConfigResponse {
    errcode: Option<i64>,
    errmsg: Option<String>,
    list: Option<Vec<McpConfigItem>>,
}

#[derive(Debug, Deserialize)]
struct McpConfigItem {
    url: Option<String>,
    biz_type: Option<String>,
    is_authed: Option<bool>,
}

struct CachedConfig {
    /// biz_type -> URL
    categories: HashMap<String, String>,
    /// biz_type -> is_authed
    auth_status: HashMap<String, bool>,
    fetched_at: Instant,
}

// ---------------------------------------------------------------------------
// Global cache
// ---------------------------------------------------------------------------

static MCP_CONFIG_CACHE: LazyLock<DashMap<String, CachedConfig>> =
    LazyLock::new(DashMap::new);

const CONFIG_TTL_SECS: u64 = 86_400; // 24 hours

const MCP_CONFIG_ENDPOINT: &str =
    "https://qyapi.weixin.qq.com/cgi-bin/aibot/cli/get_mcp_config";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get the MCP endpoint URL for a category, fetching config if needed.
///
/// Returns `Err("MCP_NO_CREDENTIALS")` if the tenant has no bot credentials.
/// Returns `Err("MCP_AUTH_EXPIRED")` if the fetched config has no URL for this category.
pub fn get_category_url(
    tenant_name: &str,
    category: &str,
    bot_id: &str,
    bot_secret: &str,
    http: &Client,
) -> Result<String, String> {
    // Check cache
    if let Some(entry) = MCP_CONFIG_CACHE.get(tenant_name) {
        if entry.fetched_at.elapsed().as_secs() < CONFIG_TTL_SECS {
            if let Some(url) = entry.categories.get(category) {
                return Ok(url.clone());
            }
            if let Some(authed) = entry.auth_status.get(category) {
                if !authed {
                    return Err(format!(
                        "MCP category '{}' is not authorized for this bot. Please enable '{0}' permission in WeChat Work admin console.",
                        category
                    ));
                }
            }
            return Err(format!(
                "MCP category '{}' not found in config for tenant '{}'",
                category, tenant_name
            ));
        }
    }

    // Fetch fresh config
    let (categories, auth_status) = fetch_config(http, bot_id, bot_secret)?;

    let url = categories
        .get(category)
        .ok_or_else(|| {
            if let Some(authed) = auth_status.get(category) {
                if !authed {
                    return format!(
                        "MCP category '{}' is not authorized for this bot. Please enable '{0}' permission in WeChat Work admin console.",
                        category
                    );
                }
            }
            format!(
                "MCP category '{}' not available. Available: {}",
                category,
                categories.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?
        .clone();

    // Cache it
    MCP_CONFIG_CACHE.insert(
        tenant_name.to_string(),
        CachedConfig {
            categories,
            auth_status,
            fetched_at: Instant::now(),
        },
    );

    Ok(url)
}

/// Invalidate cached config for a tenant (e.g., on auth error).
pub fn invalidate_cache(tenant_name: &str) {
    MCP_CONFIG_CACHE.remove(tenant_name);
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn fetch_config(
    http: &Client,
    bot_id: &str,
    bot_secret: &str,
) -> Result<(HashMap<String, String>, HashMap<String, bool>), String> {
    let body = auth::build_config_request(bot_id, bot_secret);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime creation failed: {e}"))?;

    let http = http.clone();
    rt.block_on(async {
        let resp = http
            .post(MCP_CONFIG_ENDPOINT)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                format!("OpenCarrier/{}", env!("CARGO_PKG_VERSION")),
            )
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("MCP config fetch failed: {e}"))?;

        let config: McpConfigResponse = resp
            .json()
            .await
            .map_err(|e| format!("MCP config parse failed: {e}"))?;

        let errcode = config.errcode.unwrap_or(0);
        if errcode != 0 {
            return Err(format!(
                "MCP config error {}: {}",
                errcode,
                config.errmsg.unwrap_or_default()
            ));
        }

        let mut categories = HashMap::new();
        let mut auth_status = HashMap::new();
        if let Some(list) = config.list {
            for item in list {
                if let Some(biz_type) = &item.biz_type {
                    let authed = item.is_authed.unwrap_or(false);
                    auth_status.insert(biz_type.clone(), authed);

                    if let Some(url) = item.url {
                        tracing::info!(
                            "MCP category '{}': url={}, is_authed={}",
                            biz_type,
                            &url[..url.len().min(60)],
                            authed
                        );
                        if !authed {
                            tracing::warn!(
                                "MCP category '{}' is NOT authorized — tool calls will fail. Enable '{0}' permission in WeChat Work admin.",
                                biz_type
                            );
                        }
                        categories.insert(biz_type.clone(), url);
                    } else {
                        tracing::warn!(
                            "MCP category '{}': no URL returned, is_authed={}",
                            biz_type,
                            authed
                        );
                    }
                }
            }
        }

        if categories.is_empty() {
            return Err("MCP config returned no categories".to_string());
        }

        Ok((categories, auth_status))
    })
}
