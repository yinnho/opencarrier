//! Xiaohongshu Creator API client.
//!
//! Handles cookie-based authentication and REST API request execution.

use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::time::Duration;

/// Creator API base URL.
pub const API_BASE: &str = "https://creator.xiaohongshu.com";

/// Maximum result size (60KB).
const MAX_RESULT_BYTES: usize = 60_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute a Xiaohongshu Creator API request (blocking wrapper).
///
/// `path` may contain `{param}` placeholders which are already substituted by the caller.
/// `method` is typically GET for read endpoints.
pub fn xhs_api_blocking(
    cookie_str: &str,
    path: &str,
    method: Method,
) -> Result<Value, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime available".to_string())?;

    let cookie_str = cookie_str.to_string();
    let path = path.to_string();

    tokio::task::block_in_place(|| {
        handle.block_on(async { xhs_api_async(&cookie_str, &path, method).await })
    })
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

async fn xhs_api_async(
    cookie_str: &str,
    path: &str,
    method: Method,
) -> Result<Value, String> {
    let http = Client::new();
    let url = format!("{API_BASE}{path}");

    let mut headers = HeaderMap::new();
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".parse().unwrap());
    headers.insert("Referer", "https://creator.xiaohongshu.com/".parse().unwrap());
    headers.insert("Origin", "https://creator.xiaohongshu.com".parse().unwrap());

    let req = http
        .request(method, &url)
        .headers(headers)
        .timeout(Duration::from_secs(30));

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Xiaohongshu API request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Xiaohongshu API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Xiaohongshu API HTTP {status}: {text}"));
    }

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Xiaohongshu API JSON parse error: {e}"))?;

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
