//! Bilibili REST API client with WBI signing.
//!
//! Handles cookie authentication, WBI parameter signing for protected endpoints,
//! and generic HTTP request execution.

use md5::{Digest, Md5};
use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const API_BASE: &str = "https://api.bilibili.com";
const MAX_RESULT_BYTES: usize = 60_000;

/// WBI mixin key permutation table.
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49,
    33, 9, 42, 19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61,
    26, 17, 0, 1, 60, 51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36,
    20, 34, 44, 52,
];

/// Cache for WBI keys: (img_key, sub_key, expires_at).
static WBI_KEY_CACHE: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);

const WBI_KEY_TTL: Duration = Duration::from_secs(600); // 10 min

/// Get the mixin key from img_key + sub_key using the permutation table.
fn get_mixin_key(img_key: &str, sub_key: &str) -> String {
    let combined = format!("{img_key}{sub_key}");
    MIXIN_KEY_ENC_TAB[..32]
        .iter()
        .map(|&i| combined.chars().nth(i).unwrap_or('\0'))
        .collect()
}

/// Sign parameters with WBI algorithm.
pub fn wbi_sign(params: &mut HashMap<String, String>, img_key: &str, sub_key: &str) {
    let mixin_key = get_mixin_key(img_key, sub_key);
    params.insert("wts".to_string(), std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string());

    // Sort params by key
    let mut sorted: Vec<(&String, &String)> = params.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);

    // Build query string (filter out !'()* special chars)
    let query: String = sorted.iter()
        .map(|(k, v)| {
            let filtered: String = v.chars()
                .filter(|c| !"!'()*".contains(*c))
                .collect();
            format!("{k}={filtered}")
        })
        .collect::<Vec<_>>()
        .join("&");

    // MD5 hash
    let mut hasher = Md5::new();
    hasher.update(format!("{query}{mixin_key}").as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    params.insert("w_rid".to_string(), hash);
}

/// Fetch WBI keys from /x/web-interface/nav.
async fn fetch_wbi_keys(cookie_str: &str) -> Result<(String, String), String> {
    let http = Client::new();
    let url = format!("{API_BASE}/x/web-interface/nav");

    let mut headers = HeaderMap::new();
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36".parse().unwrap());
    headers.insert("Referer", "https://www.bilibili.com/".parse().unwrap());

    let resp = http.get(&url).headers(headers).timeout(Duration::from_secs(10)).send().await
        .map_err(|e| format!("WBI key fetch failed: {e}"))?;

    let json: Value = resp.json().await.map_err(|e| format!("WBI key parse failed: {e}"))?;

    let img_url = json.pointer("/data/wbi_img/img_url")
        .and_then(|v| v.as_str()).unwrap_or("");
    let sub_url = json.pointer("/data/wbi_img/sub_url")
        .and_then(|v| v.as_str()).unwrap_or("");

    // Extract key from URL: https://i0.hdslb.com/bfs/wbi/XXXXXXXXX.webp → XXXXXXXXX
    let img_key = img_url.rsplit('/').next()
        .and_then(|s| s.split('.').next())
        .unwrap_or("").to_string();
    let sub_key = sub_url.rsplit('/').next()
        .and_then(|s| s.split('.').next())
        .unwrap_or("").to_string();

    if img_key.is_empty() || sub_key.is_empty() {
        return Err("Failed to extract WBI keys from nav response".to_string());
    }

    Ok((img_key, sub_key))
}

/// Get WBI keys (cached).
fn get_wbi_keys(cookie_str: &str) -> Result<(String, String), String> {
    {
        let cache = WBI_KEY_CACHE.lock().unwrap();
        if let Some((img, sub, expires)) = cache.as_ref() {
            if *expires > Instant::now() {
                return Ok((img.clone(), sub.clone()));
            }
        }
    }

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime".to_string())?;
    let cookie = cookie_str.to_string();

    let (img, sub) = tokio::task::block_in_place(|| {
        handle.block_on(fetch_wbi_keys(&cookie))
    })?;

    {
        let mut cache = WBI_KEY_CACHE.lock().unwrap();
        *cache = Some((img.clone(), sub.clone(), Instant::now() + WBI_KEY_TTL));
    }

    Ok((img, sub))
}

/// Build cookie string from tenant fields.
pub fn build_cookie(sessdata: &str, bili_jct: &str, dede_user_id: &str) -> String {
    format!("SESSDATA={sessdata}; bili_jct={bili_jct}; DedeUserID={dede_user_id}")
}

/// Execute a Bilibili API call (blocking wrapper).
pub fn bilibili_api_blocking(
    cookie_str: &str,
    method: Method,
    path: &str,
    params: &HashMap<String, String>,
    signed: bool,
) -> Result<Value, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime available".to_string())?;

    let cookie_str = cookie_str.to_string();
    let method = method.clone();
    let path = path.to_string();

    let mut params = params.clone();
    if signed {
        let (img_key, sub_key) = get_wbi_keys(cookie_str.as_str())?;
        wbi_sign(&mut params, &img_key, &sub_key);
    }

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            bilibili_api_async(&cookie_str, method, &path, &params).await
        })
    })
}

async fn bilibili_api_async(
    cookie_str: &str,
    method: Method,
    path: &str,
    params: &HashMap<String, String>,
) -> Result<Value, String> {
    let http = Client::new();

    let query: String = params.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    let url = if query.is_empty() {
        format!("{API_BASE}{path}")
    } else {
        format!("{API_BASE}{path}?{query}")
    };

    let mut headers = HeaderMap::new();
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36".parse().unwrap());
    headers.insert("Referer", "https://www.bilibili.com/".parse().unwrap());

    let resp = http
        .request(method, &url)
        .headers(headers)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Bilibili API request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Bilibili API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Bilibili API HTTP {status}: {}", &text[..text.len().min(500)]));
    }

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Bilibili API JSON parse error: {e}"))?;

    Ok(json)
}

/// Get current user's UID from cookies.
pub fn get_self_uid_blocking(cookie_str: &str) -> Result<u64, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime".to_string())?;
    let cookie = cookie_str.to_string();

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            let params = HashMap::new();
            let result = bilibili_api_async(&cookie, Method::GET, "/x/web-interface/nav", &params).await?;
            let uid = result.pointer("/data/mid")
                .and_then(|v| v.as_u64())
                .ok_or("Not logged in or missing mid".to_string())?;
            Ok(uid)
        })
    })
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
