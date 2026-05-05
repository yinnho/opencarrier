//! Zhihu REST API client (async) with BigInt ID fix.

use mcp_common::api::ApiClient;
use reqwest::Method;
use serde_json::Value;

const API_BASE: &str = "https://www.zhihu.com";

fn client(cookie_str: &str) -> ApiClient {
    ApiClient::new(API_BASE)
        .with_header("Cookie", cookie_str)
        .with_header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .with_header("Referer", "https://www.zhihu.com/")
}

pub async fn zhihu_api(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
) -> Result<Value, String> {
    let text = client(cookie_str)
        .request_text(method, path, query, None)
        .await?;

    // Fix BigInt IDs in JSON (Zhihu returns 16+ digit IDs as bare numbers)
    let text = regex_lite::Regex::new(r#"("id"\s*:\s*)(\d{16,})"#)
        .ok()
        .map(|re: regex_lite::Regex| re.replace_all(&text, r#"$1"$2""#).to_string())
        .unwrap_or(text);

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Zhihu API JSON parse error: {e}"))?;

    Ok(json)
}
