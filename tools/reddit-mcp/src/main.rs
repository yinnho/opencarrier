//! reddit-mcp — Reddit MCP Server (multi-tenant)
//!
//! Each tool call carries `cookie` and optionally `username`, allowing a single
//! MCP server to serve multiple Reddit accounts simultaneously.

use anyhow::Result;
use mcp_common::cookie::make_cookie;
use mcp_common::json::json_to_string;
use mcp_common::{define_params, impl_cookie, json::url_encode};
use reqwest::Method;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, transport::stdio as stdio_transport, ServiceExt};
use serde_json::Value;

mod api;

// ================================================================== //
//  Parameter structs                                                   //
// ================================================================== //

define_params!(HotParams {
    #[schemars(description = "Subreddit 名称（不含 r/），留空则为全站热门")]
    subreddit: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(HotParams);

define_params!(FrontpageParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(FrontpageParams);

define_params!(PopularParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(PopularParams);

define_params!(SubredditParams {
    #[schemars(description = "Subreddit 名称（不含 r/）")]
    name: String,
    #[schemars(description = "")]
    sort: Option<String>,
    #[schemars(description = "")]
    time: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(SubredditParams);

define_params!(SearchParams {
    #[schemars(description = "搜索关键词")]
    query: String,
    #[schemars(description = "限定 Subreddit（不含 r/）")]
    subreddit: Option<String>,
    #[schemars(description = "")]
    sort: Option<String>,
    #[schemars(description = "")]
    time: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(SearchParams);

define_params!(ReadParams {
    #[schemars(description = "帖子 ID（如 t3_xxxxx 或 xxxxx）")]
    post_id: String,
    #[schemars(description = "")]
    sort: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
    #[schemars(description = "")]
    depth: Option<i64>,
});
impl_cookie!(ReadParams);

define_params!(UserParams {
    #[schemars(description = "用户名")]
    username: String,
});
impl_cookie!(UserParams);

define_params!(UserPostsParams {
    #[schemars(description = "用户名")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(UserPostsParams);

define_params!(UserCommentsParams {
    #[schemars(description = "用户名")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(UserCommentsParams);

define_params!(SavedParams {
    #[schemars(description = "Reddit username (optional, will be resolved from cookie if not provided)")]
    username: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(SavedParams);

define_params!(UpvotedParams {
    #[schemars(description = "Reddit username (optional, will be resolved from cookie if not provided)")]
    username: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(UpvotedParams);

define_params!(UpvoteParams {
    #[schemars(description = "帖子/评论 ID（如 t3_xxxxx 或 t1_xxxxx）")]
    post_id: String,
    #[schemars(description = "")]
    direction: Option<String>,
});
impl_cookie!(UpvoteParams);

define_params!(CommentParams {
    #[schemars(description = "父级帖子/评论 ID（如 t3_xxxxx 或 t1_xxxxx）")]
    post_id: String,
    #[schemars(description = "评论内容（支持 Markdown）")]
    text: String,
});
impl_cookie!(CommentParams);

define_params!(SaveParams {
    #[schemars(description = "帖子 ID（如 t3_xxxxx）")]
    post_id: String,
    #[schemars(description = "")]
    undo: Option<bool>,
});
impl_cookie!(SaveParams);

define_params!(SubscribeParams {
    #[schemars(description = "Subreddit 名称（不含 r/）")]
    subreddit: String,
    #[schemars(description = "")]
    undo: Option<bool>,
});
impl_cookie!(SubscribeParams);

// ================================================================== //
//  Helpers                                                             //
// ================================================================== //

fn parse_post(data: &Value) -> Value {
    serde_json::json!({
        "title": data.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "subreddit": data.get("subreddit_name_prefixed").and_then(|v| v.as_str()).unwrap_or(""),
        "author": data.get("author").and_then(|v| v.as_str()).unwrap_or("[deleted]"),
        "score": data.get("score").and_then(|v| v.as_i64()).unwrap_or(0),
        "num_comments": data.get("num_comments").and_then(|v| v.as_i64()).unwrap_or(0),
        "permalink": data.get("permalink").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "url": data.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        "selftext": data.get("selftext").and_then(|v| v.as_str()).unwrap_or(""),
        "created_utc": data.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0),
    })
}

fn parse_comment(data: &Value) -> Value {
    serde_json::json!({
        "author": data.get("author").and_then(|v| v.as_str()).unwrap_or("[deleted]"),
        "body": data.get("body").and_then(|v| v.as_str()).unwrap_or("[deleted]"),
        "score": data.get("score").and_then(|v| v.as_i64()).unwrap_or(0),
        "created_utc": data.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "replies": extract_replies(data),
    })
}

fn extract_replies(data: &Value) -> Vec<Value> {
    let replies_data = data.get("replies");
    match replies_data {
        Some(replies) if replies.is_object() => {
            let children = replies
                .get("data")
                .and_then(|d| d.get("children"))
                .and_then(|c| c.as_array());
            match children {
                Some(arr) => arr
                    .iter()
                    .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("t1"))
                    .filter_map(|c| c.get("data"))
                    .map(parse_comment)
                    .collect(),
                None => vec![],
            }
        }
        _ => vec![],
    }
}

fn extract_posts_from_listing(resp: &Value) -> Vec<Value> {
    resp.get("data")
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .map(|children| {
            children
                .iter()
                .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("t3"))
                .filter_map(|c| c.get("data"))
                .map(parse_post)
                .collect()
        })
        .unwrap_or_default()
}

fn extract_comments_from_listing(resp: &Value) -> Vec<Value> {
    resp.get("data")
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .map(|children| {
            children
                .iter()
                .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("t1"))
                .filter_map(|c| c.get("data"))
                .map(parse_comment)
                .collect()
        })
        .unwrap_or_default()
}

async fn resolve_username(cookie: &str, username: Option<&str>) -> Result<String, String> {
    if let Some(u) = username {
        return Ok(u.to_string());
    }
    api::get_username(cookie).await
}

// ================================================================== //
//  MCP Server                                                          //
// ================================================================== //

#[derive(Clone)]
struct RedditServer;

#[tool_router(server_handler)]
impl RedditServer {
    // ====== READ TOOLS ======

    #[tool(description = "获取 Reddit 热门帖子")]
    async fn reddit_hot(&self, Parameters(params): Parameters<HotParams>) -> String {
        let sub = params.subreddit.as_deref().unwrap_or("").trim();
        let path = if sub.is_empty() {
            "/hot.json".to_string()
        } else {
            format!("/r/{sub}/hot.json")
        };
        let limit = params.limit.unwrap_or(20);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 全站热门 (r/all)")]
    async fn reddit_frontpage(&self, Parameters(params): Parameters<FrontpageParams>) -> String {
        let limit = params.limit.unwrap_or(15);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, "/r/all.json", Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit Popular 帖子")]
    async fn reddit_popular(&self, Parameters(params): Parameters<PopularParams>) -> String {
        let limit = params.limit.unwrap_or(20);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, "/r/popular.json", Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Subreddit 帖子列表")]
    async fn reddit_subreddit(&self, Parameters(params): Parameters<SubredditParams>) -> String {
        let sort = params.sort.as_deref().unwrap_or("hot");
        let path = format!("/r/{}/{sort}.json", params.name);
        let limit = params.limit.unwrap_or(15);
        let time = params.time.as_deref().unwrap_or("all");
        let query = format!("limit={limit}&t={time}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "搜索 Reddit 帖子")]
    async fn reddit_search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let sub = params.subreddit.as_deref().unwrap_or("").trim();
        let path = if sub.is_empty() {
            "/search.json".to_string()
        } else {
            format!("/r/{sub}/search.json")
        };
        let sort = params.sort.as_deref().unwrap_or("relevance");
        let time = params.time.as_deref().unwrap_or("all");
        let limit = params.limit.unwrap_or(15);
        let encoded_query = url_encode(&params.query);
        let mut q = format!("q={encoded_query}&sort={sort}&t={time}&limit={limit}");
        if !sub.is_empty() {
            q.push_str("&restrict_sr=on");
        }
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&q), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 帖子详情及评论")]
    async fn reddit_read(&self, Parameters(params): Parameters<ReadParams>) -> String {
        let post_id = params.post_id.strip_prefix("t3_").unwrap_or(&params.post_id);
        let path = format!("/comments/{post_id}.json");
        let sort = params.sort.as_deref().unwrap_or("best");
        let limit = params.limit.unwrap_or(25);
        let depth = params.depth.unwrap_or(2);
        let query = format!("sort={sort}&limit={limit}&depth={depth}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => {
                let arr = resp.as_array();
                let result = match arr {
                    Some(parts) if parts.len() >= 2 => {
                        let post_data = extract_posts_from_listing(&parts[0]);
                        let comments = extract_comments_from_listing(&parts[1]);
                        serde_json::json!({
                            "post": post_data.first().cloned().unwrap_or(serde_json::Value::Null),
                            "comments": comments,
                        })
                    }
                    _ => serde_json::json!({"error": "Unexpected response format"}),
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 用户资料")]
    async fn reddit_user(&self, Parameters(params): Parameters<UserParams>) -> String {
        let path = format!("/user/{}/about.json", params.username);
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, None, None).await {
            Ok(resp) => {
                let data = resp.get("data");
                let result = match data {
                    Some(d) => serde_json::json!({
                        "name": d.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "link_karma": d.get("link_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "comment_karma": d.get("comment_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "total_karma": d.get("total_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "created_utc": d.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "is_gold": d.get("is_gold").and_then(|v| v.as_bool()).unwrap_or(false),
                        "verified": d.get("verified").and_then(|v| v.as_bool()).unwrap_or(false),
                    }),
                    None => serde_json::json!({"error": "User not found"}),
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 用户帖子")]
    async fn reddit_user_posts(&self, Parameters(params): Parameters<UserPostsParams>) -> String {
        let path = format!("/user/{}/submitted.json", params.username);
        let limit = params.limit.unwrap_or(15);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 用户评论")]
    async fn reddit_user_comments(&self, Parameters(params): Parameters<UserCommentsParams>) -> String {
        let path = format!("/user/{}/comments.json", params.username);
        let limit = params.limit.unwrap_or(15);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_comments_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 收藏列表")]
    async fn reddit_saved(&self, Parameters(params): Parameters<SavedParams>) -> String {
        let username = match resolve_username(&make_cookie(&params), params.username.as_deref()).await {
            Ok(u) => u,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let path = format!("/user/{username}/saved.json");
        let limit = params.limit.unwrap_or(15);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Reddit 点赞列表")]
    async fn reddit_upvoted(&self, Parameters(params): Parameters<UpvotedParams>) -> String {
        let username = match resolve_username(&make_cookie(&params), params.username.as_deref()).await {
            Ok(u) => u,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let path = format!("/user/{username}/upvoted.json");
        let limit = params.limit.unwrap_or(15);
        let query = format!("limit={limit}");
        match api::reddit_api(&make_cookie(&params), Method::GET, &path, Some(&query), None).await {
            Ok(resp) => json_to_string(&serde_json::Value::Array(extract_posts_from_listing(&resp))),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ====== WRITE TOOLS ======

    #[tool(description = "对 Reddit 帖子/评论投票")]
    async fn reddit_upvote(&self, Parameters(params): Parameters<UpvoteParams>) -> String {
        let cookie = make_cookie(&params);
        let modhash = match api::get_modhash(&cookie).await {
            Ok(m) => m,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let post_id = &params.post_id;
        let dir = match params.direction.as_deref().unwrap_or("up") {
            "up" => "1",
            "down" => "-1",
            _ => "0",
        };
        let body = format!("id={post_id}&dir={dir}&uh={modhash}");
        match api::reddit_api(&cookie, Method::POST, "/api/vote", None, Some(&body)).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("success").and_then(|v| v.as_bool()).unwrap_or(true)})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "在 Reddit 帖子/评论下发表评论")]
    async fn reddit_comment(&self, Parameters(params): Parameters<CommentParams>) -> String {
        let cookie = make_cookie(&params);
        let modhash = match api::get_modhash(&cookie).await {
            Ok(m) => m,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let post_id = &params.post_id;
        let encoded_text = url_encode(&params.text);
        let body = format!("parent={post_id}&text={encoded_text}&uh={modhash}");
        match api::reddit_api(&cookie, Method::POST, "/api/comment", None, Some(&body)).await {
            Ok(resp) => {
                let success = resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                json_to_string(&serde_json::json!({"ok": success}))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "收藏/取消收藏 Reddit 帖子")]
    async fn reddit_save(&self, Parameters(params): Parameters<SaveParams>) -> String {
        let cookie = make_cookie(&params);
        let modhash = match api::get_modhash(&cookie).await {
            Ok(m) => m,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let post_id = &params.post_id;
        let undo = params.undo.unwrap_or(false);
        let path = if undo { "/api/unsave" } else { "/api/save" };
        let body = format!("id={post_id}&uh={modhash}");
        match api::reddit_api(&cookie, Method::POST, path, None, Some(&body)).await {
            Ok(_) => json_to_string(&serde_json::json!({"ok": true})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "订阅/取消订阅 Subreddit")]
    async fn reddit_subscribe(&self, Parameters(params): Parameters<SubscribeParams>) -> String {
        let cookie = make_cookie(&params);
        let modhash = match api::get_modhash(&cookie).await {
            Ok(m) => m,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };
        let subreddit = &params.subreddit;
        let undo = params.undo.unwrap_or(false);
        let action = if undo { "unsub" } else { "sub" };
        let body = format!("sr_name={subreddit}&action={action}&uh={modhash}");
        match api::reddit_api(&cookie, Method::POST, "/api/subscribe", None, Some(&body)).await {
            Ok(_) => json_to_string(&serde_json::json!({"ok": true})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }
}

// ================================================================== //
//  Entry point                                                         //
// ================================================================== //

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("REDDIT_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    tracing::info!("reddit-mcp starting (stdio, multi-tenant)");
    let server = RedditServer;
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;

    Ok(())
}
