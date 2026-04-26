//! Twitter/X GraphQL API client.
//!
//! Handles authentication (ct0 + auth_token cookies + Bearer token),
//! queryId resolution, and generic GraphQL request execution.

use crate::TWITTER_TENANTS;
use reqwest::{header::HeaderMap, Client, Method};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Twitter's public web app Bearer token (constant, same for all users).
const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

/// GraphQL API base URL.
const API_BASE: &str = "https://x.com";

/// Maximum result size (60KB).
const MAX_RESULT_BYTES: usize = 60_000;

/// QueryId cache: operation_name → (query_id, expires_at).
static QUERY_ID_CACHE: Mutex<Option<HashMap<String, (String, Instant)>>> =
    Mutex::new(None);

/// Cache TTL for queryIds (1 hour).
const QUERY_ID_TTL: Duration = Duration::from_secs(3600);

/// Fallback queryIds (valid as of 2026-04, may need updating).
static FALLBACK_QUERY_IDS: &[(&str, &str)] = &[
    ("UserByScreenName", "qRednkZG-rn1P6b48NINmQ"),
    ("Likes", "RozQdCp4CilQzrcuU0NY5w"),
    ("Bookmarks", "Fy0QMy4q_aZCpkO0PnyLYw"),
    ("UserTweets", "6fWQaBPK51aGyC_VC7t9GQ"),
    ("HomeTimeline", "c-CzHF1LboFilMpsx4ZCrQ"),
    ("HomeLatestTimeline", "BKB7oi212Fi7kQtCBGE4zA"),
    ("TweetDetail", "nBS-WpgA6ZG0CyNHD517JQ"),
    ("TweetResultByRestId", "7xflPyRiUxGVbJd4uWmbfg"),
    ("ListsManagementPageTimeline", "78UbkyXwXBD98IgUWXOy9g"),
    ("ListLatestTweetsTimeline", "RlZzktZY_9wJynoepm8ZsA"),
    ("SearchTimeline", "UN1i3zUiCWa-6r-Uaho4fw"),
    ("Followers", "d_J4iBqGgbpE-PNVBLtIcw"),
    ("Following", "nV_F5woCqYQmOqXnCk0BBw"),
    ("NotificationsTimeline", "B9_KmbkLhXt6jRwGjJrweg"),
    ("FavoriteTweet", "lI07N6OtwuG07WN68viA-w"),
    ("UnfavoriteTweet", "ZYGCSeEiDHtbVKDXkMBjJA"),
    ("CreateBookmark", "aoK7MWCEd1ta1X9qNmIjyw"),
    ("DestroyBookmark", "W9VWQOE0ICqyVr0wFSxzEg"),
    ("CreateFollow", "TQ2guLZiNNZBJf6KVuLJYw"),
    ("DestroyFollow", "DfAsaKxryYxvE-T7agQVSg"),
    ("CreateTweet", "jejOjFeLQqVOypeCqVkk1g"),
    ("DeleteTweet", "VaenaCGRD5tXZqMKXIMB2g"),
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get cookie string for a tenant.
#[allow(dead_code)]
pub fn get_cookies(tenant_id: &str) -> Result<String, String> {
    let entry = TWITTER_TENANTS
        .get(tenant_id)
        .ok_or_else(|| {
            let names: Vec<String> = TWITTER_TENANTS.iter().map(|e| e.key().clone()).collect();
            format!(
                "Unknown Twitter tenant '{}'. Available tenants: {}",
                tenant_id,
                names.join(", ")
            )
        })?;
    Ok(format!("ct0={}; auth_token={}", entry.value().ct0, entry.value().auth_token))
}

/// Get ct0 value (CSRF token) for a tenant.
#[allow(dead_code)]
pub fn get_csrf_token(tenant_id: &str) -> Result<String, String> {
    let entry = TWITTER_TENANTS
        .get(tenant_id)
        .ok_or_else(|| format!("Unknown Twitter tenant '{}'", tenant_id))?;
    Ok(entry.value().ct0.clone())
}

/// Resolve a queryId for a GraphQL operation name.
/// Tries cached value first, then fetches from GitHub, then falls back to hardcoded.
pub fn resolve_query_id(operation_name: &str) -> String {
    // Check cache
    {
        let cache = QUERY_ID_CACHE.lock().unwrap();
        if let Some(map) = cache.as_ref() {
            if let Some((qid, expires)) = map.get(operation_name) {
                if *expires > Instant::now() {
                    return qid.clone();
                }
            }
        }
    }

    // Try fetching from GitHub via tokio runtime
    let github_qid = tokio::runtime::Handle::try_current().ok().and_then(|handle| {
        let op = operation_name.to_string();
        handle.block_on(async { fetch_query_id_from_github(&op).await.ok() })
    });

    if let Some(qid) = github_qid {
        // Cache it
        {
            let mut cache = QUERY_ID_CACHE.lock().unwrap();
            let map = cache.get_or_insert_with(HashMap::new);
            map.insert(operation_name.to_string(), (qid.clone(), Instant::now() + QUERY_ID_TTL));
        }
        return qid;
    }

    // Fallback to hardcoded
    FALLBACK_QUERY_IDS
        .iter()
        .find(|(op, _)| *op == operation_name)
        .map(|(_, qid)| qid.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Execute a Twitter GraphQL query/mutation.
pub fn twitter_graphql_blocking(
    cookie_str: &str,
    csrf_token: &str,
    query_id: &str,
    operation_name: &str,
    variables: &Value,
    features: &Value,
    method: Method,
) -> Result<Value, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "No tokio runtime available".to_string())?;

    let cookie_str = cookie_str.to_string();
    let csrf_token = csrf_token.to_string();
    let query_id = query_id.to_string();
    let operation_name = operation_name.to_string();
    let variables = variables.clone();
    let features = features.clone();

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            twitter_graphql_async(
                &cookie_str,
                &csrf_token,
                &query_id,
                &operation_name,
                &variables,
                &features,
                method,
            )
            .await
        })
    })
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

async fn fetch_query_id_from_github(operation_name: &str) -> Result<String, String> {
    let url = "https://raw.githubusercontent.com/fa0311/twitter-openapi/refs/heads/main/src/config/placeholder.json";
    let http = Client::new();
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("GitHub fetch failed: {e}"))?;

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("GitHub JSON parse failed: {e}"))?;

    json.get(operation_name)
        .and_then(|v| v.get("queryId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("queryId not found for operation: {operation_name}"))
}

async fn twitter_graphql_async(
    cookie_str: &str,
    csrf_token: &str,
    query_id: &str,
    operation_name: &str,
    variables: &Value,
    features: &Value,
    method: Method,
) -> Result<Value, String> {
    let http = Client::new();
    let url = format!("{API_BASE}/i/api/graphql/{query_id}/{operation_name}");

    let mut headers = HeaderMap::new();
    headers.insert("Authorization", format!("Bearer {BEARER_TOKEN}").parse().unwrap());
    headers.insert("X-Csrf-Token", csrf_token.parse().unwrap());
    headers.insert("X-Twitter-Auth-Type", "OAuth2Session".parse().unwrap());
    headers.insert("X-Twitter-Active-User", "yes".parse().unwrap());
    headers.insert("Cookie", cookie_str.parse().unwrap());
    headers.insert("Content-Type", "application/json".parse().unwrap());

    let req = if method == Method::POST {
        let body = serde_json::json!({
            "variables": variables,
            "features": features,
            "queryId": query_id,
        });
        http.request(method, &url)
            .headers(headers)
            .json(&body)
            .timeout(Duration::from_secs(30))
    } else {
        // GET: variables and features as query params
        let query = format!(
            "variables={}&features={}",
            urlencoding(variables),
            urlencoding(features),
        );
        http.request(method, format!("{url}?{query}"))
            .headers(headers)
            .timeout(Duration::from_secs(30))
    };

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Twitter API request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Twitter API read body failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Twitter API HTTP {status}: {text}"));
    }

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Twitter API JSON parse error: {e}"))?;

    Ok(json)
}

/// URL-encode a JSON value for query parameter usage.
fn urlencoding(val: &Value) -> String {
    // Simple percent-encoding for JSON strings in query params.
    let s = serde_json::to_string(val).unwrap_or_default();
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
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

/// Standard Twitter GraphQL features object (used by most read endpoints).
pub fn standard_features() -> Value {
    serde_json::json!({
        "rweb_video_screen_enabled": false,
        "profile_label_improvements_pcf_label_in_post_enabled": true,
        "rweb_tipjar_consumption_enabled": true,
        "verified_phone_label_enabled": false,
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "premium_content_api_read_enabled": false,
        "communities_web_enable_tweet_community_results_fetch": true,
        "c9s_tweet_anatomy_moderator_badge_enabled": true,
        "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
        "responsive_web_grok_analyze_post_followups_enabled": true,
        "responsive_web_jetfuel_frame": false,
        "responsive_web_grok_share_attachment_enabled": true,
        "articles_preview_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": true,
        "tweet_awards_web_tipping_enabled": false,
        "responsive_web_grok_show_grok_translated_post": false,
        "responsive_web_grok_analysis_button_from_backend": false,
        "creator_subscriptions_quote_tweet_preview_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_grok_image_annotation_enabled": true,
        "responsive_web_enhance_cards_enabled": false
    })
}
