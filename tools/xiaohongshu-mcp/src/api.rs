//! Xiaohongshu Creator API client (async).

use mcp_common::api::ApiClient;
use reqwest::Method;
use serde_json::Value;

const API_BASE: &str = "https://creator.xiaohongshu.com";

fn client(cookie_str: &str) -> ApiClient {
    ApiClient::new(API_BASE)
        .with_header("Cookie", cookie_str)
        .with_header("Content-Type", "application/json")
        .with_header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .with_header("Referer", "https://creator.xiaohongshu.com/")
        .with_header("Origin", "https://creator.xiaohongshu.com")
}

pub async fn xhs_api(
    cookie_str: &str,
    path: &str,
    method: Method,
) -> Result<Value, String> {
    client(cookie_str)
        .request(method, path, None, None)
        .await
}
