//! Generic Feishu/Lark REST API caller.
//!
//! All tool implementations use `feishu_api()` to make authenticated requests
//! against the Feishu Open API. Token management is handled by TenantTokenCache.

use crate::plugin::channels::feishu::token::TenantTokenCache;
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;

/// Maximum result size (60KB, leaving 4KB margin for the 64KB FFI buffer).
pub const MAX_RESULT_BYTES: usize = 60_000;

/// Generic Feishu API call.
///
/// Automatically:
/// - Prepends `token_cache.api_base()` to `path`
/// - Adds `Authorization: Bearer {token}`
/// - Handles Feishu error codes
/// - 30s timeout
pub async fn feishu_api(
    http: &Client,
    token_cache: &TenantTokenCache,
    method: Method,
    path: &str,
    query: Option<&Value>,
    body: Option<&Value>,
) -> Result<Value, String> {
    let token = token_cache.get_token().await?;
    let base = token_cache.api_base().to_string();
    let url = format!("{base}/{path}");

    let mut req = http
        .request(method, &url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(30));

    // Add query parameters
    if let Some(q) = query {
        if let Some(obj) = q.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    req = req.query(&[(k, s)]);
                } else if !v.is_null() {
                    req = req.query(&[(k, v.to_string())]);
                }
            }
        }
    }

    // Add body
    if let Some(b) = body {
        req = req.json(b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Feishu API request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Feishu API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Feishu API HTTP {status}: {text}"));
    }

    // Parse JSON
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Feishu API JSON parse error: {e}"))?;

    // Check Feishu error code
    let code = json.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 0 {
        let msg = json.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown");
        return Err(format!("Feishu API error: code={code} msg={msg}"));
    }

    Ok(json)
}

/// Call feishu_api using the synchronous blocking pattern (for built-in plugin context).
pub fn feishu_api_blocking(
    http: &Client,
    token_cache: &TenantTokenCache,
    method: Method,
    path: &str,
    query: Option<&Value>,
    body: Option<&Value>,
) -> Result<Value, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime creation failed: {e}"))?;

    rt.block_on(async {
        feishu_api(http, token_cache, method, path, query, body).await
    })
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
