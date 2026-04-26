//! Zhihu REST API client.
//!
//! Handles cookie-based authentication and generic HTTP requests.

use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::time::Duration;

const API_BASE: &str = "https://www.zhihu.com";
const MAX_RESULT_BYTES: usize = 60_000;

pub fn zhihu_api_blocking(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
) -> Result<Value, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime available".to_string())?;

    let cookie_str = cookie_str.to_string();
    let method = method.clone();
    let path = path.to_string();
    let query = query.map(|s| s.to_string());

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            zhihu_api_async(&cookie_str, method, &path, query.as_deref()).await
        })
    })
}

async fn zhihu_api_async(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
) -> Result<Value, String> {
    let http = Client::new();
    let url = if let Some(q) = query {
        format!("{API_BASE}{path}?{q}")
    } else {
        format!("{API_BASE}{path}")
    };

    let mut headers = HeaderMap::new();
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36".parse().unwrap());
    headers.insert("Referer", "https://www.zhihu.com/".parse().unwrap());

    let resp = http
        .request(method, &url)
        .headers(headers)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Zhihu API request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Zhihu API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Zhihu API HTTP {status}: {}", &text[..text.len().min(500)]));
    }

    // Fix BigInt IDs in JSON (Zhihu returns 16+ digit IDs as bare numbers)
    let text = regex_lite::Regex::new(r#"("id"\s*:\s*)(\d{16,})"#)
        .ok()
        .map(|re: regex_lite::Regex| re.replace_all(&text, r#"$1"$2""#).to_string())
        .unwrap_or(text);

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Zhihu API JSON parse error: {e}"))?;

    Ok(json)
}

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
