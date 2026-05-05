//! Reddit REST API client (async) with modhash support.

use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::time::Duration;

const API_BASE: &str = "https://www.reddit.com";

pub async fn reddit_api(
    cookie_str: &str,
    method: Method,
    path: &str,
    query: Option<&str>,
    body: Option<&str>,
) -> Result<Value, String> {
    let http = Client::new();

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

pub async fn get_modhash(cookie_str: &str) -> Result<String, String> {
    let result = reddit_api(cookie_str, Method::GET, "/api/me.json", None, None).await?;
    result
        .get("data")
        .and_then(|d| d.get("modhash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Failed to get modhash from /api/me.json".to_string())
}

pub async fn get_username(cookie_str: &str) -> Result<String, String> {
    let result = reddit_api(cookie_str, Method::GET, "/api/me.json", None, None).await?;
    result
        .get("data")
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Failed to get username from /api/me.json".to_string())
}
