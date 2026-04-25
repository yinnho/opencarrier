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
}

struct CachedConfig {
    /// biz_type -> URL
    categories: HashMap<String, String>,
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
            return Err(format!(
                "MCP category '{}' not found in config for tenant '{}'",
                category, tenant_name
            ));
        }
    }

    // Fetch fresh config
    let categories = fetch_config(http, bot_id, bot_secret)?;

    let url = categories
        .get(category)
        .ok_or_else(|| {
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
) -> Result<HashMap<String, String>, String> {
    let body = auth::build_config_request(bot_id, bot_secret);

    let rt = tokio::runtime::Handle::current();
    let http = http.clone();
    tokio::task::block_in_place(|| {
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
            if let Some(list) = config.list {
                for item in list {
                    if let (Some(url), Some(biz_type)) = (item.url, item.biz_type) {
                        categories.insert(biz_type, url);
                    }
                }
            }

            if categories.is_empty() {
                return Err("MCP config returned no categories".to_string());
            }

            Ok(categories)
        })
    })
}
