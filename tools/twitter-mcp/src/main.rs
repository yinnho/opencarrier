//! twitter-mcp — Twitter/X MCP Server (multi-tenant)
//!
//! Each tool call carries `ct0` and `auth_token`, allowing a single MCP server
//! to serve multiple Twitter/X accounts simultaneously.

use anyhow::Result;
use mcp_common::json::json_to_string;
use reqwest::Method;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, transport::stdio as stdio_transport, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

mod api;

// ================================================================== //
//  Macros                                                              //
// ================================================================== //

macro_rules! define_params {
    ($name:ident { $($field:tt)* }) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        struct $name {
            #[schemars(description = "Twitter ct0 cookie (CSRF token)")]
            ct0: String,
            #[schemars(description = "Twitter auth_token cookie")]
            auth_token: String,
            $($field)*
        }
    };
}

trait TokenHolder {
    fn ct0(&self) -> &str;
    fn auth_token(&self) -> &str;
}

macro_rules! impl_tokens {
    ($type:ty) => {
        impl TokenHolder for $type {
            fn ct0(&self) -> &str {
                &self.ct0
            }
            fn auth_token(&self) -> &str {
                &self.auth_token
            }
        }
    };
}

// ================================================================== //
//  Parameter structs                                                   //
// ================================================================== //

define_params!(SearchParams {
    #[schemars(description = "搜索关键词")]
    query: String,
    #[schemars(description = "")]
    filter: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(SearchParams);

define_params!(TimelineParams {
    #[schemars(description = "")]
    r#type: Option<String>,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(TimelineParams);

define_params!(TweetsParams {
    #[schemars(description = "用户名（不含@）")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(TweetsParams);

define_params!(ProfileParams {
    #[schemars(description = "用户名（不含@）")]
    username: String,
});
impl_tokens!(ProfileParams);

define_params!(FollowersParams {
    #[schemars(description = "用户名")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(FollowersParams);

define_params!(FollowingParams {
    #[schemars(description = "用户名")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(FollowingParams);

define_params!(ThreadParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(ThreadParams);

define_params!(BookmarksParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(BookmarksParams);

define_params!(LikesParams {
    #[schemars(description = "用户名")]
    username: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(LikesParams);

define_params!(ListsParams {});
impl_tokens!(ListsParams);

define_params!(ListTweetsParams {
    #[schemars(description = "列表ID")]
    list_id: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(ListTweetsParams);

define_params!(NotificationsParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_tokens!(NotificationsParams);

define_params!(ArticleParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(ArticleParams);

define_params!(LikeParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(LikeParams);

define_params!(UnlikeParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(UnlikeParams);

define_params!(BookmarkParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(BookmarkParams);

define_params!(FollowParams {
    #[schemars(description = "用户名")]
    username: String,
});
impl_tokens!(FollowParams);

define_params!(UnfollowParams {
    #[schemars(description = "用户名")]
    username: String,
});
impl_tokens!(UnfollowParams);

define_params!(PostParams {
    #[schemars(description = "推文内容")]
    text: String,
});
impl_tokens!(PostParams);

define_params!(DeleteParams {
    #[schemars(description = "推文ID")]
    tweet_id: String,
});
impl_tokens!(DeleteParams);

// ================================================================== //
//  Helpers                                                             //
// ================================================================== //

fn make_cookie<T: TokenHolder>(params: &T) -> String {
    format!("ct0={}; auth_token={}", params.ct0(), params.auth_token())
}

async fn gql<T: TokenHolder>(
    params: &T,
    operation: &str,
    variables: Value,
    features: Option<Value>,
    method: Method,
) -> Result<Value, String> {
    let query_id = api::resolve_query_id(operation).await;
    let features = features.unwrap_or_else(api::standard_features);
    api::twitter_graphql(
        &make_cookie(params),
        params.ct0(),
        &query_id,
        operation,
        &variables,
        &features,
        method,
    )
    .await
}

// ---------------------------------------------------------------------------
// Tweet parsing helpers
// ---------------------------------------------------------------------------

fn parse_tweet(result: &Value) -> Option<Value> {
    let tw = if result.get("__typename").and_then(|v| v.as_str()) == Some("TweetWithVisibilityResults") {
        result.get("tweet")?
    } else {
        result
    };

    let rest_id = tw.get("rest_id")?.as_str()?.to_string();
    let user = tw.get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|u| u.get("result"));

    let screen_name = user
        .and_then(|u| {
            u.get("core").and_then(|c| c.get("screen_name")).or_else(|| {
                u.get("legacy").and_then(|l| l.get("screen_name"))
            })
        })
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let name = user
        .and_then(|u| u.get("legacy").and_then(|l| l.get("name")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let legacy = tw.get("legacy");
    let text = tw.get("note_tweet")
        .and_then(|n| n.get("note_tweet_results"))
        .and_then(|n| n.get("result"))
        .and_then(|r| r.get("text"))
        .or_else(|| legacy.and_then(|l| l.get("full_text")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let likes = legacy
        .and_then(|l| l.get("favorite_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let retweets = legacy
        .and_then(|l| l.get("retweet_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let views = tw.get("views")
        .and_then(|v| v.get("count"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    let created_at = legacy
        .and_then(|l| l.get("created_at"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let is_retweet = legacy
        .and_then(|l| l.get("retweeted_status_id_str"))
        .is_some();

    let media = legacy
        .and_then(|l| l.get("extended_entities").and_then(|e| e.get("media")))
        .or_else(|| legacy.and_then(|l| l.get("entities").and_then(|e| e.get("media"))));

    let media_urls: Vec<String> = media
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter().filter_map(|m| {
                let mtype = m.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if mtype == "video" || mtype == "animated_gif" {
                    m.get("video_info")
                        .and_then(|vi| vi.get("variants"))
                        .and_then(|v| v.as_array())
                        .and_then(|vars| {
                            vars.iter()
                                .filter(|v| v.get("content_type").and_then(|t| t.as_str()) == Some("video/mp4"))
                                .filter_map(|v| v.get("url").and_then(|u| u.as_str()))
                                .next()
                                .map(|s| s.to_string())
                        })
                        .or_else(|| m.get("media_url_https").and_then(|u| u.as_str()).map(|s| s.to_string()))
                } else {
                    m.get("media_url_https").and_then(|u| u.as_str()).map(|s| s.to_string())
                }
            }).collect()
        })
        .unwrap_or_default();

    let has_media = !media_urls.is_empty();

    Some(serde_json::json!({
        "id": rest_id,
        "author": screen_name,
        "name": name,
        "text": text,
        "likes": likes,
        "retweets": retweets,
        "views": views,
        "created_at": created_at,
        "url": format!("https://x.com/i/status/{rest_id}"),
        "is_retweet": is_retweet,
        "has_media": has_media,
        "media_urls": media_urls,
    }))
}

fn extract_tweets_from_instructions(instructions: &Value) -> Vec<Value> {
    let mut tweets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let insts = match instructions.as_array() {
        Some(a) => a,
        None => return tweets,
    };

    for inst in insts {
        let entries = inst.get("entries").or_else(|| {
            if inst.get("type").and_then(|t| t.as_str()) == Some("TimelineAddEntries") {
                inst.get("entries")
            } else {
                None
            }
        });

        let entries = match entries.and_then(|e| e.as_array()) {
            Some(e) => e,
            None => continue,
        };

        for entry in entries {
            let entry_id = entry.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
            if !entry_id.starts_with("tweet-") {
                continue;
            }

            let result = entry
                .get("content")
                .and_then(|c| c.get("itemContent"))
                .and_then(|i| i.get("tweet_results"))
                .and_then(|t| t.get("result"));

            if let Some(tweet) = result.and_then(|r| parse_tweet(r)) {
                let id = tweet.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if seen.insert(id.clone()) {
                    tweets.push(tweet);
                }
                continue;
            }

            let items = entry
                .get("content")
                .and_then(|c| c.get("items"))
                .and_then(|i| i.as_array());

            if let Some(items) = items {
                for item in items {
                    let result = item
                        .get("item")
                        .and_then(|i| i.get("itemContent"))
                        .and_then(|i| i.get("tweet_results"))
                        .and_then(|t| t.get("result"));

                    if let Some(tweet) = result.and_then(|r| parse_tweet(r)) {
                        let id = tweet.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if seen.insert(id.clone()) {
                            tweets.push(tweet);
                        }
                    }
                }
            }
        }
    }

    tweets
}

// ================================================================== //
//  MCP Server                                                          //
// ================================================================== //

#[derive(Clone)]
struct TwitterServer;

#[tool_router(server_handler)]
impl TwitterServer {
    // ====== READ TOOLS ======

    #[tool(description = "搜索 Twitter/X 推文")]
    async fn twitter_search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let filter = params.filter.as_deref().unwrap_or("top");
        let variables = serde_json::json!({
            "rawQuery": params.query,
            "count": params.limit.unwrap_or(15),
            "querySource": "typed_query",
            "product": if filter == "live" { "Latest" } else { "Top" }
        });
        match gql(&params, "SearchTimeline", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/search_by_raw_query/search_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 时间线")]
    async fn twitter_timeline(&self, Parameters(params): Parameters<TimelineParams>) -> String {
        let timeline_type = params.r#type.as_deref().unwrap_or("for-you");
        let mut variables = serde_json::json!({
            "count": params.limit.unwrap_or(20),
            "includePromotedContent": false,
            "latestControlAvailable": true,
            "requestContext": "launch",
        });
        if timeline_type == "for-you" {
            variables["withCommunity"] = serde_json::json!(true);
        }
        let op = if timeline_type == "following" { "HomeLatestTimeline" } else { "HomeTimeline" };
        match gql(&params, op, variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/home/home_timeline_urt/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 用户推文列表")]
    async fn twitter_tweets(&self, Parameters(params): Parameters<TweetsParams>) -> String {
        let variables = serde_json::json!({
            "screen_name": params.username,
            "withSafetyModeUserFields": true,
        });
        match gql(&params, "UserByScreenName", variables, None, Method::GET).await {
            Ok(resp) => {
                let user = resp.pointer("/data/user/result");
                let result = if let Some(user) = user {
                    let legacy = user.get("legacy");
                    serde_json::json!({
                        "screen_name": user.get("legacy").and_then(|l| l.get("screen_name")),
                        "name": legacy.as_ref().and_then(|l| l.get("name")),
                        "bio": legacy.as_ref().and_then(|l| l.get("description")),
                        "followers": legacy.as_ref().and_then(|l| l.get("followers_count")),
                        "following": legacy.as_ref().and_then(|l| l.get("friends_count")),
                        "tweets_count": legacy.as_ref().and_then(|l| l.get("statuses_count")),
                        "note": "Use twitter_timeline for actual tweet list. This returns user profile as userId resolution."
                    })
                } else {
                    serde_json::json!({"error": "User not found"})
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 用户资料")]
    async fn twitter_profile(&self, Parameters(params): Parameters<ProfileParams>) -> String {
        let variables = serde_json::json!({
            "screen_name": params.username,
            "withSafetyModeUserFields": true,
        });
        match gql(&params, "UserByScreenName", variables, None, Method::GET).await {
            Ok(resp) => {
                let user = resp.pointer("/data/user/result").cloned().unwrap_or(serde_json::Value::Null);
                let legacy = user.get("legacy");
                let result = serde_json::json!({
                    "screen_name": user.get("legacy").and_then(|l| l.get("screen_name")),
                    "name": legacy.as_ref().and_then(|l| l.get("name")),
                    "bio": legacy.as_ref().and_then(|l| l.get("description")),
                    "location": legacy.as_ref().and_then(|l| l.get("location")),
                    "url": legacy.as_ref().and_then(|l| l.get("url")),
                    "followers": legacy.as_ref().and_then(|l| l.get("followers_count")),
                    "following": legacy.as_ref().and_then(|l| l.get("friends_count")),
                    "tweets_count": legacy.as_ref().and_then(|l| l.get("statuses_count")),
                    "likes_count": legacy.as_ref().and_then(|l| l.get("favourites_count")),
                    "verified": user.get("is_blue_verified").or_else(|| legacy.as_ref().and_then(|l| l.get("verified"))),
                    "created_at": legacy.as_ref().and_then(|l| l.get("created_at")),
                });
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 用户粉丝列表")]
    async fn twitter_followers(&self, Parameters(params): Parameters<FollowersParams>) -> String {
        let variables = serde_json::json!({
            "screen_name": params.username,
            "withSafetyModeUserFields": true,
        });
        match gql(&params, "Followers", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/user/result/timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                let mut users = Vec::new();
                if let Some(entries) = instructions.as_array() {
                    for inst in entries {
                        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let entry_id = entry.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
                                if !entry_id.starts_with("user-") { continue; }
                                let user = entry.get("content")
                                    .and_then(|c| c.get("itemContent"))
                                    .and_then(|i| i.get("user_results"))
                                    .and_then(|u| u.get("result"));
                                if let Some(u) = user {
                                    users.push(serde_json::json!({
                                        "screen_name": u.get("legacy").and_then(|l| l.get("screen_name")),
                                        "name": u.get("legacy").and_then(|l| l.get("name")),
                                        "bio": u.get("legacy").and_then(|l| l.get("description")),
                                        "followers": u.get("legacy").and_then(|l| l.get("followers_count")),
                                    }));
                                }
                            }
                        }
                    }
                }
                json_to_string(&serde_json::Value::Array(users))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 用户关注列表")]
    async fn twitter_following(&self, Parameters(params): Parameters<FollowingParams>) -> String {
        let variables = serde_json::json!({
            "screen_name": params.username,
            "withSafetyModeUserFields": true,
        });
        match gql(&params, "Following", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/user/result/timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                let mut users = Vec::new();
                if let Some(entries) = instructions.as_array() {
                    for inst in entries {
                        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let entry_id = entry.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
                                if !entry_id.starts_with("user-") { continue; }
                                let user = entry.get("content")
                                    .and_then(|c| c.get("itemContent"))
                                    .and_then(|i| i.get("user_results"))
                                    .and_then(|u| u.get("result"));
                                if let Some(u) = user {
                                    users.push(serde_json::json!({
                                        "screen_name": u.get("legacy").and_then(|l| l.get("screen_name")),
                                        "name": u.get("legacy").and_then(|l| l.get("name")),
                                        "bio": u.get("legacy").and_then(|l| l.get("description")),
                                        "followers": u.get("legacy").and_then(|l| l.get("followers_count")),
                                    }));
                                }
                            }
                        }
                    }
                }
                json_to_string(&serde_json::Value::Array(users))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 推文线程")]
    async fn twitter_thread(&self, Parameters(params): Parameters<ThreadParams>) -> String {
        let variables = serde_json::json!({
            "focalTweetId": params.tweet_id,
            "referrer": "tweet",
            "with_rux_injections": false,
            "includePromotedContent": false,
            "rankingMode": "Recency",
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true,
        });
        match gql(&params, "TweetDetail", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/threaded_conversation_with_injections_v2/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 收藏列表")]
    async fn twitter_bookmarks(&self, Parameters(params): Parameters<BookmarksParams>) -> String {
        let variables = serde_json::json!({
            "count": params.limit.unwrap_or(20),
            "includePromotedContent": false,
        });
        match gql(&params, "Bookmarks", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/bookmark_timeline_v2/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 用户点赞列表")]
    async fn twitter_likes(&self, Parameters(params): Parameters<LikesParams>) -> String {
        let variables = serde_json::json!({
            "screen_name": params.username,
            "withSafetyModeUserFields": true,
        });
        match gql(&params, "Likes", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/user/result/timeline_v2/timeline/instructions")
                    .or_else(|| resp.pointer("/data/user/result/timeline/timeline/instructions"))
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 列表")]
    async fn twitter_lists(&self, Parameters(params): Parameters<ListsParams>) -> String {
        match gql(&params, "ListsManagementPageTimeline", serde_json::json!({}), None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/viewer/list_management_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                let mut lists = Vec::new();
                if let Some(insts) = instructions.as_array() {
                    for inst in insts {
                        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let list = entry.get("content")
                                    .and_then(|c| c.get("itemContent"))
                                    .and_then(|i| i.get("list_results"))
                                    .and_then(|l| l.get("result"));
                                if let Some(l) = list {
                                    lists.push(serde_json::json!({
                                        "id": l.get("id_str"),
                                        "name": l.get("legacy").and_then(|l| l.get("name")),
                                        "members": l.get("legacy").and_then(|l| l.get("member_count")),
                                        "followers": l.get("legacy").and_then(|l| l.get("subscriber_count")),
                                        "mode": l.get("legacy").and_then(|l| l.get("mode")),
                                    }));
                                }
                            }
                        }
                    }
                }
                json_to_string(&serde_json::Value::Array(lists))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 列表推文")]
    async fn twitter_list_tweets(&self, Parameters(params): Parameters<ListTweetsParams>) -> String {
        let variables = serde_json::json!({
            "listId": params.list_id,
            "count": params.limit.unwrap_or(20),
        });
        match gql(&params, "ListLatestTweetsTimeline", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/list/tweets_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                json_to_string(&serde_json::Value::Array(extract_tweets_from_instructions(&instructions)))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 通知")]
    async fn twitter_notifications(&self, Parameters(params): Parameters<NotificationsParams>) -> String {
        let variables = serde_json::json!({
            "count": params.limit.unwrap_or(20),
            "includePromotedContent": false,
        });
        match gql(&params, "NotificationsTimeline", variables, None, Method::GET).await {
            Ok(resp) => {
                let instructions = resp.pointer("/data/viewer/timeline_response/timeline/instructions")
                    .or_else(|| resp.pointer("/data/viewer_v2/user_results/result/notification_timeline/timeline/instructions"))
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                let mut notifications = Vec::new();
                if let Some(insts) = instructions.as_array() {
                    for inst in insts {
                        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let entry_id = entry.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
                                if !entry_id.starts_with("notification-") { continue; }
                                let content = entry.get("content").and_then(|c| c.get("itemContent"));
                                if let Some(c) = content {
                                    notifications.push(serde_json::json!({
                                        "id": entry_id,
                                        "type": c.get("displayTreatment").and_then(|d| d.get("actionText")),
                                        "message": c.get("header").and_then(|h| h.get("text")),
                                    }));
                                }
                            }
                        }
                    }
                }
                json_to_string(&serde_json::Value::Array(notifications))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取 Twitter/X 长文内容")]
    async fn twitter_article(&self, Parameters(params): Parameters<ArticleParams>) -> String {
        let variables = serde_json::json!({
            "tweetId": params.tweet_id,
            "withCommunity": false,
            "includePromotedContent": false,
            "withVoice": false,
        });
        let features = serde_json::json!({
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "responsive_web_enhance_cards_enabled": false,
        });
        match gql(&params, "TweetResultByRestId", variables, Some(features), Method::GET).await {
            Ok(resp) => {
                let article = resp.pointer("/data/tweetResult/result/article/article_results/result");
                let result = if let Some(a) = article {
                    serde_json::json!({
                        "title": a.get("title"),
                        "author": a.get("core").and_then(|c| c.get("user_results")).and_then(|u| u.get("result")).and_then(|r| r.get("legacy")).and_then(|l| l.get("screen_name")),
                        "url": a.get("url"),
                    })
                } else {
                    serde_json::json!({"error": "Article not found"})
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ====== WRITE TOOLS ======

    #[tool(description = "点赞推文")]
    async fn twitter_like(&self, Parameters(params): Parameters<LikeParams>) -> String {
        let variables = serde_json::json!({"tweet_id": params.tweet_id});
        match gql(&params, "FavoriteTweet", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "取消点赞推文")]
    async fn twitter_unlike(&self, Parameters(params): Parameters<UnlikeParams>) -> String {
        let variables = serde_json::json!({"tweet_id": params.tweet_id});
        match gql(&params, "UnfavoriteTweet", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "收藏推文")]
    async fn twitter_bookmark(&self, Parameters(params): Parameters<BookmarkParams>) -> String {
        let variables = serde_json::json!({"tweet_id": params.tweet_id});
        match gql(&params, "CreateBookmark", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "关注用户")]
    async fn twitter_follow(&self, Parameters(params): Parameters<FollowParams>) -> String {
        let variables = serde_json::json!({"screen_name": params.username});
        match gql(&params, "CreateFollow", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "取消关注用户")]
    async fn twitter_unfollow(&self, Parameters(params): Parameters<UnfollowParams>) -> String {
        let variables = serde_json::json!({"screen_name": params.username});
        match gql(&params, "DestroyFollow", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "发布推文")]
    async fn twitter_post(&self, Parameters(params): Parameters<PostParams>) -> String {
        let variables = serde_json::json!({
            "tweet_text": params.text,
            "dark_request": false,
            "media": {"media_entities": [], "possibly_sensitive": false},
            "semantic_annotation_ids": [],
        });
        match gql(&params, "CreateTweet", variables, None, Method::POST).await {
            Ok(resp) => {
                let tweet_id = resp.pointer("/data/create_tweet/tweet_results/result/rest_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                json_to_string(&serde_json::json!({
                    "ok": true,
                    "tweet_id": tweet_id,
                    "url": format!("https://x.com/i/status/{tweet_id}")
                }))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "删除推文")]
    async fn twitter_delete(&self, Parameters(params): Parameters<DeleteParams>) -> String {
        let variables = serde_json::json!({
            "tweet_id": params.tweet_id,
            "dark_request": false,
        });
        match gql(&params, "DeleteTweet", variables, None, Method::POST).await {
            Ok(resp) => json_to_string(&serde_json::json!({"ok": resp.get("data").is_some()})),
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
            tracing_subscriber::EnvFilter::try_from_env("TWITTER_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    tracing::info!("twitter-mcp starting (stdio, multi-tenant)");
    let server = TwitterServer;
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;

    Ok(())
}
