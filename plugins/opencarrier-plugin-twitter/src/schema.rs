//! Twitter tool specifications — 20 tools backed by GraphQL API.

use reqwest::Method;
use serde_json::{json, Value};

/// A Twitter tool specification.
pub struct TwitterToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// GraphQL operation name (e.g. "SearchTimeline").
    pub operation: &'static str,
    /// HTTP method for the GraphQL call.
    pub method: Method,
    /// Build GraphQL variables from tool args.
    pub build_variables: fn(&Value) -> Value,
    /// Optional custom features override. None = use standard_features().
    pub features: Option<Value>,
    /// Extract structured result from the GraphQL response.
    pub parse_response: fn(&Value) -> Value,
}

// ---------------------------------------------------------------------------
// Shared tweet parser
// ---------------------------------------------------------------------------

/// Extract a tweet object from various response structures.
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

    // Extract media
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

    Some(json!({
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

/// Extract tweets from a timeline instructions array.
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

            // Try direct itemContent
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

            // Try nested items (conversation threads)
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

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

pub fn all_tools() -> Vec<TwitterToolSpec> {
    vec![
        // ====== READ TOOLS ======

        TwitterToolSpec {
            name: "twitter_search",
            description: "搜索 Twitter/X 推文",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "filter": {"type": "string", "description": "过滤模式：top(热门) 或 live(最新)", "default": "top"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                },
                "required": ["query"]
            }),
            operation: "SearchTimeline",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "rawQuery": args["query"],
                    "count": args["limit"].as_i64().unwrap_or(15),
                    "querySource": "typed_query",
                    "product": if args["filter"].as_str() == Some("live") { "Latest" } else { "Top" }
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/search_by_raw_query/search_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_timeline",
            description: "获取 Twitter/X 时间线",
            schema: json!({
                "type": "object",
                "properties": {
                    "type": {"type": "string", "description": "时间线类型：for-you(推荐) 或 following(关注)", "default": "for-you"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                }
            }),
            operation: "HomeTimeline",
            method: Method::GET,
            build_variables: |args| {
                let timeline_type = args["type"].as_str().unwrap_or("for-you");
                let mut vars = json!({
                    "count": args["limit"].as_i64().unwrap_or(20),
                    "includePromotedContent": false,
                    "latestControlAvailable": true,
                    "requestContext": "launch",
                });
                if timeline_type == "for-you" {
                    vars["withCommunity"] = json!(true);
                }
                vars
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/home/home_timeline_urt/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_tweets",
            description: "获取 Twitter/X 用户推文列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名（不含@）"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["username"]
            }),
            operation: "UserTweets",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "screen_name": args["username"],
                    "withSafetyModeUserFields": true,
                })
            },
            features: None,
            parse_response: |resp| {
                // Two-step: first resolve userId, then get tweets
                // For simplicity, return the user info from UserByScreenName
                let user = resp.pointer("/data/user/result");
                if let Some(user) = user {
                    let legacy = user.get("legacy");
                    json!({
                        "screen_name": user.get("legacy").and_then(|l| l.get("screen_name")),
                        "name": legacy.as_ref().and_then(|l| l.get("name")),
                        "bio": legacy.as_ref().and_then(|l| l.get("description")),
                        "followers": legacy.as_ref().and_then(|l| l.get("followers_count")),
                        "following": legacy.as_ref().and_then(|l| l.get("friends_count")),
                        "tweets_count": legacy.as_ref().and_then(|l| l.get("statuses_count")),
                        "note": "Use twitter_timeline for actual tweet list. This returns user profile as userId resolution."
                    })
                } else {
                    json!({"error": "User not found"})
                }
            },
        },

        TwitterToolSpec {
            name: "twitter_profile",
            description: "获取 Twitter/X 用户资料",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名（不含@）"}
                },
                "required": ["username"]
            }),
            operation: "UserByScreenName",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "screen_name": args["username"],
                    "withSafetyModeUserFields": true,
                })
            },
            features: None,
            parse_response: |resp| {
                let user = resp.pointer("/data/user/result").cloned().unwrap_or(json!(null));
                let legacy = user.get("legacy");
                json!({
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
                })
            },
        },

        TwitterToolSpec {
            name: "twitter_followers",
            description: "获取 Twitter/X 用户粉丝列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["username"]
            }),
            operation: "Followers",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "screen_name": args["username"],
                    "withSafetyModeUserFields": true,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/user/result/timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
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
                                    users.push(json!({
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
                json!(users)
            },
        },

        TwitterToolSpec {
            name: "twitter_following",
            description: "获取 Twitter/X 用户关注列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["username"]
            }),
            operation: "Following",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "screen_name": args["username"],
                    "withSafetyModeUserFields": true,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/user/result/timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
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
                                    users.push(json!({
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
                json!(users)
            },
        },

        TwitterToolSpec {
            name: "twitter_thread",
            description: "获取 Twitter/X 推文线程",
            schema: json!({
                "type": "object",
                "properties": {
                    "tweet_id": {"type": "string", "description": "推文ID"}
                },
                "required": ["tweet_id"]
            }),
            operation: "TweetDetail",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "focalTweetId": args["tweet_id"],
                    "referrer": "tweet",
                    "with_rux_injections": false,
                    "includePromotedContent": false,
                    "rankingMode": "Recency",
                    "withCommunity": true,
                    "withQuickPromoteEligibilityTweetFields": true,
                    "withBirdwatchNotes": true,
                    "withVoice": true,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/threaded_conversation_with_injections_v2/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_bookmarks",
            description: "获取 Twitter/X 收藏列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                }
            }),
            operation: "Bookmarks",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "count": args["limit"].as_i64().unwrap_or(20),
                    "includePromotedContent": false,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/bookmark_timeline_v2/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_likes",
            description: "获取 Twitter/X 用户点赞列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "username": {"type": "string", "description": "用户名"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["username"]
            }),
            operation: "Likes",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "screen_name": args["username"],
                    "withSafetyModeUserFields": true,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/user/result/timeline_v2/timeline/instructions")
                    .or_else(|| resp.pointer("/data/user/result/timeline/timeline/instructions"))
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_lists",
            description: "获取 Twitter/X 列表",
            schema: json!({"type": "object", "properties": {}}),
            operation: "ListsManagementPageTimeline",
            method: Method::GET,
            build_variables: |_| json!({}),
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/viewer/list_management_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
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
                                    lists.push(json!({
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
                json!(lists)
            },
        },

        TwitterToolSpec {
            name: "twitter_list_tweets",
            description: "获取 Twitter/X 列表推文",
            schema: json!({
                "type": "object",
                "properties": {
                    "list_id": {"type": "string", "description": "列表ID"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["list_id"]
            }),
            operation: "ListLatestTweetsTimeline",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "listId": args["list_id"],
                    "count": args["limit"].as_i64().unwrap_or(20),
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/list/tweets_timeline/timeline/instructions")
                    .cloned()
                    .unwrap_or(json!([]));
                json!(extract_tweets_from_instructions(&instructions))
            },
        },

        TwitterToolSpec {
            name: "twitter_notifications",
            description: "获取 Twitter/X 通知",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                }
            }),
            operation: "NotificationsTimeline",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "count": args["limit"].as_i64().unwrap_or(20),
                    "includePromotedContent": false,
                })
            },
            features: None,
            parse_response: |resp| {
                let instructions = resp.pointer("/data/viewer/timeline_response/timeline/instructions")
                    .or_else(|| resp.pointer("/data/viewer_v2/user_results/result/notification_timeline/timeline/instructions"))
                    .cloned()
                    .unwrap_or(json!([]));
                let mut notifications = Vec::new();
                if let Some(insts) = instructions.as_array() {
                    for inst in insts {
                        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let entry_id = entry.get("entryId").and_then(|v| v.as_str()).unwrap_or("");
                                if !entry_id.starts_with("notification-") { continue; }
                                let content = entry.get("content").and_then(|c| c.get("itemContent"));
                                if let Some(c) = content {
                                    notifications.push(json!({
                                        "id": entry_id,
                                        "type": c.get("displayTreatment").and_then(|d| d.get("actionText")),
                                        "message": c.get("header").and_then(|h| h.get("text")),
                                    }));
                                }
                            }
                        }
                    }
                }
                json!(notifications)
            },
        },

        TwitterToolSpec {
            name: "twitter_article",
            description: "获取 Twitter/X 长文内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "tweet_id": {"type": "string", "description": "推文ID"}
                },
                "required": ["tweet_id"]
            }),
            operation: "TweetResultByRestId",
            method: Method::GET,
            build_variables: |args| {
                json!({
                    "tweetId": args["tweet_id"],
                    "withCommunity": false,
                    "includePromotedContent": false,
                    "withVoice": false,
                })
            },
            features: Some(json!({
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
            })),
            parse_response: |resp| {
                let article = resp.pointer("/data/tweetResult/result/article/article_results/result");
                if let Some(a) = article {
                    json!({
                        "title": a.get("title"),
                        "author": a.get("core").and_then(|c| c.get("user_results")).and_then(|u| u.get("result")).and_then(|r| r.get("legacy")).and_then(|l| l.get("screen_name")),
                        "url": a.get("url"),
                    })
                } else {
                    json!({"error": "Article not found"})
                }
            },
        },

        // ====== WRITE TOOLS (mutations) ======

        TwitterToolSpec {
            name: "twitter_like",
            description: "点赞推文",
            schema: json!({
                "type": "object",
                "properties": {"tweet_id": {"type": "string", "description": "推文ID"}},
                "required": ["tweet_id"]
            }),
            operation: "FavoriteTweet",
            method: Method::POST,
            build_variables: |args| json!({"tweet_id": args["tweet_id"]}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },

        TwitterToolSpec {
            name: "twitter_unlike",
            description: "取消点赞推文",
            schema: json!({
                "type": "object",
                "properties": {"tweet_id": {"type": "string", "description": "推文ID"}},
                "required": ["tweet_id"]
            }),
            operation: "UnfavoriteTweet",
            method: Method::POST,
            build_variables: |args| json!({"tweet_id": args["tweet_id"]}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },

        TwitterToolSpec {
            name: "twitter_bookmark",
            description: "收藏推文",
            schema: json!({
                "type": "object",
                "properties": {"tweet_id": {"type": "string", "description": "推文ID"}},
                "required": ["tweet_id"]
            }),
            operation: "CreateBookmark",
            method: Method::POST,
            build_variables: |args| json!({"tweet_id": args["tweet_id"]}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },

        TwitterToolSpec {
            name: "twitter_follow",
            description: "关注用户",
            schema: json!({
                "type": "object",
                "properties": {"username": {"type": "string", "description": "用户名"}},
                "required": ["username"]
            }),
            operation: "CreateFollow",
            method: Method::POST,
            build_variables: |args| json!({"screen_name": args["username"]}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },

        TwitterToolSpec {
            name: "twitter_unfollow",
            description: "取消关注用户",
            schema: json!({
                "type": "object",
                "properties": {"username": {"type": "string", "description": "用户名"}},
                "required": ["username"]
            }),
            operation: "DestroyFollow",
            method: Method::POST,
            build_variables: |args| json!({"screen_name": args["username"]}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },

        TwitterToolSpec {
            name: "twitter_post",
            description: "发布推文",
            schema: json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "推文内容"}},
                "required": ["text"]
            }),
            operation: "CreateTweet",
            method: Method::POST,
            build_variables: |args| {
                json!({
                    "tweet_text": args["text"],
                    "dark_request": false,
                    "media": {"media_entities": [], "possibly_sensitive": false},
                    "semantic_annotation_ids": [],
                })
            },
            features: None,
            parse_response: |resp| {
                let tweet_id = resp.pointer("/data/create_tweet/tweet_results/result/rest_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                json!({"ok": true, "tweet_id": tweet_id, "url": format!("https://x.com/i/status/{tweet_id}")})
            },
        },

        TwitterToolSpec {
            name: "twitter_delete",
            description: "删除推文",
            schema: json!({
                "type": "object",
                "properties": {"tweet_id": {"type": "string", "description": "推文ID"}},
                "required": ["tweet_id"]
            }),
            operation: "DeleteTweet",
            method: Method::POST,
            build_variables: |args| json!({"tweet_id": args["tweet_id"], "dark_request": false}),
            features: None,
            parse_response: |resp| json!({"ok": resp.get("data").is_some()}),
        },
    ]
}
