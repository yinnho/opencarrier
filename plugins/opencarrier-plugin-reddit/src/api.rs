//! Reddit REST API client.
//!
//! Handles authentication (cookie-based session), modhash retrieval,
//! and generic REST request execution.

use crate::REDDIT_TENANTS;
use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::time::Duration;

/// Reddit API base URL.
const API_BASE: &str = "https://www.reddit.com";

/// Maximum result size (60KB).
const MAX_RESULT_BYTES: usize = 60_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get cookie string for a tenant.
#[allow(dead_code)]
pub fn get_cookies(tenant_id: &str) -> Result<String, String> {
    let entry = REDDIT_TENANTS
        .get(tenant_id)
        .ok_or_else(|| {
            let names: Vec<String> = REDDIT_TENANTS.iter().map(|e| e.key().clone()).collect();
            format!(
                "Unknown Reddit tenant '{}'. Available tenants: {}",
                tenant_id,
                names.join(", ")
            )
        })?;
    Ok(entry.value().cookie.clone())
}

/// Get configured username for a tenant (may be None).
pub fn get_configured_username(tenant_id: &str) -> Option<String> {
    REDDIT_TENANTS
        .get(tenant_id)
        .and_then(|e| e.value().username.clone())
}

/// Fetch modhash (CSRF token) for authenticated POST requests.
pub fn get_modhash_blocking(cookie_str: &str) -> Result<String, String> {
    let result = reddit_api_blocking(cookie_str, Method::GET, "/api/me.json", None, None)?;
    result
        .get("data")
        .and_then(|d| d.get("modhash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Failed to get modhash from /api/me.json".to_string())
}

/// Fetch authenticated username from /api/me.json.
pub fn get_username_blocking(cookie_str: &str) -> Result<String, String> {
    let result = reddit_api_blocking(cookie_str, Method::GET, "/api/me.json", None, None)?;
    result
        .get("data")
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Failed to get username from /api/me.json".to_string())
}

/// Execute a Reddit API request (blocking wrapper).
///
/// - GET: appends query params and `&raw_json=1`
/// - POST: sends form-urlencoded body with Content-Type header
pub fn reddit_api_blocking(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
    body: Option<&str>,
) -> Result<Value, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime available".to_string())?;

    let cookie_str = cookie_str.to_string();
    let path = path.to_string();
    let query = query.map(|s| s.to_string());
    let body = body.map(|s| s.to_string());

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            reddit_api_async(&cookie_str, method, &path, query.as_deref(), body.as_deref()).await
        })
    })
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

async fn reddit_api_async(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
    body: Option<&str>,
) -> Result<Value, String> {
    let http = Client::new();

    // Build URL with query params
    let url = if method == Method::GET {
        let mut url = format!("{API_BASE}{path}");
        if let Some(q) = query {
            if !q.is_empty() {
                url.push_str(&format!("?{q}&raw_json=1"));
            } else {
                url.push_str("?raw_json=1");
            }
        } else {
            url.push_str("?raw_json=1");
        }
        url
    } else {
        format!("{API_BASE}{path}")
    };

    let mut headers = HeaderMap::new();
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("User-Agent", "OpenCarrier/0.1.0".parse().unwrap());

    let req = if method == Method::POST {
        headers.insert(
            "Content-Type",
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let body_text = body.unwrap_or("");
        http.request(method, &url)
            .headers(headers)
            .body(body_text.to_string())
            .timeout(Duration::from_secs(30))
    } else {
        http.request(method, &url)
            .headers(headers)
            .timeout(Duration::from_secs(30))
    };

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Reddit API request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Reddit API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Reddit API HTTP {status}: {text}"));
    }

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Reddit API JSON parse error: {e}"))?;

    Ok(json)
}

/// Truncate result to fit within the FFI buffer.
pub fn truncate_result(text: String) -> String {
    if text.len() > MAX_RESULT_BYTES {
        let truncated = &text[..MAX_RESULT_BYTES];
        let boundary = truncated
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(MAX_RESULT_BYTES);
        format!(
            "{}...\n(truncated, full result is {} bytes)",
            &text[..boundary],
            text.len()
        )
    } else {
        text
    }
}
