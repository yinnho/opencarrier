//! Reddit tool specifications — 15 tools backed by Reddit REST API.

use reqwest::Method;
use serde_json::{json, Value};

/// A Reddit tool specification.
pub struct RedditToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// HTTP method for the API call.
    pub method: Method,
    /// Build the API path from tool args (e.g. "/r/{name}/hot.json").
    pub build_path: fn(&Value) -> String,
    /// Build query string from tool args (e.g. "limit=20&sort=hot").
    pub build_query: fn(&Value) -> String,
    /// Build POST body from tool args (form-urlencoded). None for GET requests.
    pub build_body: Option<fn(&Value, &str) -> String>,
    /// Whether this tool requires a modhash (write tools).
    pub needs_modhash: bool,
    /// Whether this tool needs to resolve username first (saved/upvoted).
    pub needs_username: bool,
    /// Extract structured result from the API response.
    pub parse_response: fn(&Value) -> Value,
}

// ---------------------------------------------------------------------------
// Shared post parser
// ---------------------------------------------------------------------------

/// Extract a post object from Reddit listing data.
pub fn parse_post(data: &Value) -> Value {
    json!({
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

/// Extract a comment object from Reddit comment data.
fn parse_comment(data: &Value) -> Value {
    json!({
        "author": data.get("author").and_then(|v| v.as_str()).unwrap_or("[deleted]"),
        "body": data.get("body").and_then(|v| v.as_str()).unwrap_or("[deleted]"),
        "score": data.get("score").and_then(|v| v.as_i64()).unwrap_or(0),
        "created_utc": data.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "replies": extract_replies(data),
    })
}

/// Recursively extract replies from a comment.
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

/// Extract posts from a Reddit listing response.
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

/// Extract comments from a Reddit listing response.
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

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

pub fn all_tools() -> Vec<RedditToolSpec> {
    vec![
        // ====== READ TOOLS ======

        // 1. reddit_hot
        RedditToolSpec {
            name: "reddit_hot",
            description: "获取 Reddit 热门帖子",
            schema: json!({
                "type": "object",
                "properties": {
                    "subreddit": {"type": "string", "description": "Subreddit 名称（不含 r/），留空则为全站热门"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": []
            }),
            method: Method::GET,
            build_path: |args| {
                let sub = args["subreddit"].as_str().unwrap_or("").trim();
                if sub.is_empty() {
                    "/hot.json".to_string()
                } else {
                    format!("/r/{sub}/hot.json")
                }
            },
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(20);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 2. reddit_frontpage
        RedditToolSpec {
            name: "reddit_frontpage",
            description: "获取 Reddit 全站热门 (r/all)",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": []
            }),
            method: Method::GET,
            build_path: |_args| "/r/all.json".to_string(),
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 3. reddit_popular
        RedditToolSpec {
            name: "reddit_popular",
            description: "获取 Reddit Popular 帖子",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": []
            }),
            method: Method::GET,
            build_path: |_args| "/r/popular.json".to_string(),
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(20);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 4. reddit_subreddit
        RedditToolSpec {
            name: "reddit_subreddit",
            description: "获取 Subreddit 帖子列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Subreddit 名称（不含 r/）"},
                    "sort": {"type": "string", "description": "排序方式", "default": "hot", "enum": ["hot","new","top","rising"]},
                    "time": {"type": "string", "description": "时间范围（仅 top 有效）", "default": "all", "enum": ["hour","day","week","month","year","all"]},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": ["name"]
            }),
            method: Method::GET,
            build_path: |args| {
                let name = args["name"].as_str().unwrap_or("");
                let sort = args["sort"].as_str().unwrap_or("hot");
                format!("/r/{name}/{sort}.json")
            },
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                let time = args["time"].as_str().unwrap_or("all");
                format!("limit={limit}&t={time}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 5. reddit_search
        RedditToolSpec {
            name: "reddit_search",
            description: "搜索 Reddit 帖子",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "subreddit": {"type": "string", "description": "限定 Subreddit（不含 r/）"},
                    "sort": {"type": "string", "description": "排序方式", "default": "relevance", "enum": ["relevance","hot","top","new","comments"]},
                    "time": {"type": "string", "description": "时间范围", "default": "all", "enum": ["hour","day","week","month","year","all"]},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": ["query"]
            }),
            method: Method::GET,
            build_path: |args| {
                let sub = args["subreddit"].as_str().unwrap_or("").trim();
                if sub.is_empty() {
                    "/search.json".to_string()
                } else {
                    format!("/r/{sub}/search.json")
                }
            },
            build_query: |args| {
                let query = args["query"].as_str().unwrap_or("");
                let sort = args["sort"].as_str().unwrap_or("relevance");
                let time = args["time"].as_str().unwrap_or("all");
                let limit = args["limit"].as_i64().unwrap_or(15);
                let sub = args["subreddit"].as_str().unwrap_or("").trim();
                let encoded_query = url_encode(query);
                let mut q = format!("q={encoded_query}&sort={sort}&t={time}&limit={limit}");
                if !sub.is_empty() {
                    q.push_str("&restrict_sr=on");
                }
                q
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 6. reddit_read (post detail + comments)
        RedditToolSpec {
            name: "reddit_read",
            description: "获取 Reddit 帖子详情及评论",
            schema: json!({
                "type": "object",
                "properties": {
                    "post_id": {"type": "string", "description": "帖子 ID（如 t3_xxxxx 或 xxxxx）"},
                    "sort": {"type": "string", "description": "评论排序", "default": "best", "enum": ["best","top","new","controversial","old","qa"]},
                    "limit": {"type": "integer", "description": "评论数量", "default": 25},
                    "depth": {"type": "integer", "description": "评论深度", "default": 2}
                },
                "required": ["post_id"]
            }),
            method: Method::GET,
            build_path: |args| {
                let post_id = args["post_id"].as_str().unwrap_or("");
                // Strip "t3_" prefix if present
                let clean_id = post_id.strip_prefix("t3_").unwrap_or(post_id);
                format!("/comments/{clean_id}.json")
            },
            build_query: |args| {
                let sort = args["sort"].as_str().unwrap_or("best");
                let limit = args["limit"].as_i64().unwrap_or(25);
                let depth = args["depth"].as_i64().unwrap_or(2);
                format!("sort={sort}&limit={limit}&depth={depth}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| {
                // Response is an array [0]=post listing, [1]=comments listing
                let arr = resp.as_array();
                match arr {
                    Some(parts) if parts.len() >= 2 => {
                        let post_data = extract_posts_from_listing(&parts[0]);
                        let comments = extract_comments_from_listing(&parts[1]);
                        json!({
                            "post": post_data.first().cloned().unwrap_or(json!(null)),
                            "comments": comments,
                        })
                    }
                    _ => json!({"error": "Unexpected response format"}),
                }
            },
        },

        // 7. reddit_user
        RedditToolSpec {
            name: "reddit_user",
            description: "获取 Reddit 用户资料",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"}
                },
                "required": ["username"]
            }),
            method: Method::GET,
            build_path: |args| {
                let username = args["username"].as_str().unwrap_or("");
                format!("/user/{username}/about.json")
            },
            build_query: |_args| String::new(),
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| {
                let data = resp.get("data");
                match data {
                    Some(d) => json!({
                        "name": d.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "link_karma": d.get("link_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "comment_karma": d.get("comment_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "total_karma": d.get("total_karma").and_then(|v| v.as_i64()).unwrap_or(0),
                        "created_utc": d.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "is_gold": d.get("is_gold").and_then(|v| v.as_bool()).unwrap_or(false),
                        "verified": d.get("verified").and_then(|v| v.as_bool()).unwrap_or(false),
                    }),
                    None => json!({"error": "User not found"}),
                }
            },
        },

        // 8. reddit_user_posts
        RedditToolSpec {
            name: "reddit_user_posts",
            description: "获取 Reddit 用户帖子",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": ["username"]
            }),
            method: Method::GET,
            build_path: |args| {
                let username = args["username"].as_str().unwrap_or("");
                format!("/user/{username}/submitted.json")
            },
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 9. reddit_user_comments
        RedditToolSpec {
            name: "reddit_user_comments",
            description: "获取 Reddit 用户评论",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": ["username"]
            }),
            method: Method::GET,
            build_path: |args| {
                let username = args["username"].as_str().unwrap_or("");
                format!("/user/{username}/comments.json")
            },
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: false,
            parse_response: |resp| json!(extract_comments_from_listing(resp)),
        },

        // 10. reddit_saved
        RedditToolSpec {
            name: "reddit_saved",
            description: "获取 Reddit 收藏列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": []
            }),
            method: Method::GET,
            build_path: |_args| "/user/me/saved.json".to_string(),
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: true, // need to resolve username for the path
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // 11. reddit_upvoted
        RedditToolSpec {
            name: "reddit_upvoted",
            description: "获取 Reddit 点赞列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": []
            }),
            method: Method::GET,
            build_path: |_args| "/user/me/upvoted.json".to_string(),
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(15);
                format!("limit={limit}")
            },
            build_body: None,
            needs_modhash: false,
            needs_username: true,
            parse_response: |resp| json!(extract_posts_from_listing(resp)),
        },

        // ====== WRITE TOOLS (need modhash) ======

        // 12. reddit_upvote
        RedditToolSpec {
            name: "reddit_upvote",
            description: "对 Reddit 帖子/评论投票",
            schema: json!({
                "type": "object",
                "properties": {
                    "post_id": {"type": "string", "description": "帖子/评论 ID（如 t3_xxxxx 或 t1_xxxxx）"},
                    "direction": {"type": "string", "description": "投票方向", "default": "up", "enum": ["up","down","none"]}
                },
                "required": ["post_id"]
            }),
            method: Method::POST,
            build_path: |_args| "/api/vote".to_string(),
            build_query: |_args| String::new(),
            build_body: Some(|args, modhash| {
                let post_id = args["post_id"].as_str().unwrap_or("");
                let dir = match args["direction"].as_str().unwrap_or("up") {
                    "up" => "1",
                    "down" => "-1",
                    _ => "0",
                };
                format!("id={post_id}&dir={dir}&uh={modhash}")
            }),
            needs_modhash: true,
            needs_username: false,
            parse_response: |resp| json!({"ok": resp.get("success").and_then(|v| v.as_bool()).unwrap_or(true)}),
        },

        // 13. reddit_comment
        RedditToolSpec {
            name: "reddit_comment",
            description: "在 Reddit 帖子/评论下发表评论",
            schema: json!({
                "type": "object",
                "properties": {
                    "post_id": {"type": "string", "description": "父级帖子/评论 ID（如 t3_xxxxx 或 t1_xxxxx）"},
                    "text": {"type": "string", "description": "评论内容（支持 Markdown）"}
                },
                "required": ["post_id", "text"]
            }),
            method: Method::POST,
            build_path: |_args| "/api/comment".to_string(),
            build_query: |_args| String::new(),
            build_body: Some(|args, modhash| {
                let post_id = args["post_id"].as_str().unwrap_or("");
                let text = args["text"].as_str().unwrap_or("");
                let encoded_text = url_encode(text);
                format!("parent={post_id}&text={encoded_text}&uh={modhash}")
            }),
            needs_modhash: true,
            needs_username: false,
            parse_response: |resp| {
                let success = resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                json!({"ok": success})
            },
        },

        // 14. reddit_save
        RedditToolSpec {
            name: "reddit_save",
            description: "收藏/取消收藏 Reddit 帖子",
            schema: json!({
                "type": "object",
                "properties": {
                    "post_id": {"type": "string", "description": "帖子 ID（如 t3_xxxxx）"},
                    "undo": {"type": "boolean", "description": "true 为取消收藏", "default": false}
                },
                "required": ["post_id"]
            }),
            method: Method::POST,
            build_path: |args| {
                let undo = args["undo"].as_bool().unwrap_or(false);
                if undo { "/api/unsave".to_string() } else { "/api/save".to_string() }
            },
            build_query: |_args| String::new(),
            build_body: Some(|args, modhash| {
                let post_id = args["post_id"].as_str().unwrap_or("");
                format!("id={post_id}&uh={modhash}")
            }),
            needs_modhash: true,
            needs_username: false,
            parse_response: |_resp| json!({"ok": true}),
        },

        // 15. reddit_subscribe
        RedditToolSpec {
            name: "reddit_subscribe",
            description: "订阅/取消订阅 Subreddit",
            schema: json!({
                "type": "object",
                "properties": {
                    "subreddit": {"type": "string", "description": "Subreddit 名称（不含 r/）"},
                    "undo": {"type": "boolean", "description": "true 为取消订阅", "default": false}
                },
                "required": ["subreddit"]
            }),
            method: Method::POST,
            build_path: |_args| "/api/subscribe".to_string(),
            build_query: |_args| String::new(),
            build_body: Some(|args, modhash| {
                let subreddit = args["subreddit"].as_str().unwrap_or("");
                let undo = args["undo"].as_bool().unwrap_or(false);
                let action = if undo { "unsub" } else { "sub" };
                format!("sr_name={subreddit}&action={action}&uh={modhash}")
            }),
            needs_modhash: true,
            needs_username: false,
            parse_response: |_resp| json!({"ok": true}),
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple URL encoding for query/form values.
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push('+');
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}
