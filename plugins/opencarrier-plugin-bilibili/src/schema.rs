//! Bilibili tool specifications — 13 tools backed by REST API.

use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct BilibiliToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub method: Method,
    pub path: &'static str,
    pub build_params: fn(&Value) -> HashMap<String, String>,
    /// Whether this endpoint needs WBI signing.
    pub signed: bool,
    pub parse_response: fn(&Value) -> Value,
}

/// Parse a video item from search results.
fn parse_video(item: &Value) -> Value {
    json!({
        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "author": item.get("author").or_else(|| item.get("owner").and_then(|o| o.get("name"))).and_then(|v| v.as_str()).unwrap_or(""),
        "play": item.get("play").or_else(|| item.get("stat").and_then(|s| s.get("view"))).and_then(|v| v.as_i64()).unwrap_or(0),
        "danmaku": item.get("stat").and_then(|s| s.get("danmaku")).and_then(|v| v.as_i64()).unwrap_or(0),
        "likes": item.get("stat").and_then(|s| s.get("like")).and_then(|v| v.as_i64()).unwrap_or(0),
        "bvid": item.get("bvid").and_then(|v| v.as_str()).unwrap_or(""),
        "url": item.get("bvid").and_then(|v| v.as_str()).map(|b| format!("https://www.bilibili.com/video/{b}")).unwrap_or_default(),
    })
}

/// Parse a dynamic/feed item.
fn parse_dynamic_item(item: &Value) -> Option<Value> {
    let author = item.pointer("/modules/module_author/name")
        .and_then(|v| v.as_str()).unwrap_or("");
    let id_str = item.get("id_str").and_then(|v| v.as_str()).unwrap_or("");

    // Try different content types
    let desc = item.pointer("/modules/module_dynamic/desc/text")
        .and_then(|v| v.as_str()).unwrap_or("");
    let archive_title = item.pointer("/modules/module_dynamic/major/archive/title")
        .and_then(|v| v.as_str()).unwrap_or("");
    let archive_url = item.pointer("/modules/module_dynamic/major/archive/jump_url")
        .and_then(|v| v.as_str()).unwrap_or("");

    let likes = item.pointer("/modules/module_stat/like/count")
        .and_then(|v| v.as_i64()).unwrap_or(0);
    let comments = item.pointer("/modules/module_stat/comment/count")
        .and_then(|v| v.as_i64()).unwrap_or(0);

    Some(json!({
        "id": id_str,
        "author": author,
        "title": if archive_title.is_empty() { desc.to_string() } else { archive_title.to_string() },
        "url": if archive_url.starts_with("//") { format!("https:{archive_url}") } else { archive_url.to_string() },
        "likes": likes,
        "comments": comments,
    }))
}

pub fn all_tools() -> Vec<BilibiliToolSpec> {
    vec![
        // ====== READ TOOLS ======

        BilibiliToolSpec {
            name: "bilibili_video",
            description: "获取视频信息",
            schema: json!({
                "type": "object",
                "properties": { "bvid": {"type": "string", "description": "视频 BV 号"} },
                "required": ["bvid"]
            }),
            method: Method::GET,
            path: "/x/web-interface/view",
            build_params: |args| {
                let mut m = HashMap::new();
                if let Some(bvid) = args["bvid"].as_str() {
                    m.insert("bvid".into(), bvid.into());
                }
                m
            },
            signed: false,
            parse_response: |resp| {
                let d = resp.pointer("/data");
                if let Some(d) = d {
                    json!({
                        "bvid": d.get("bvid"),
                        "aid": d.get("aid"),
                        "title": d.get("title"),
                        "author": d.pointer("/owner/name"),
                        "author_mid": d.pointer("/owner/mid"),
                        "category": d.get("tname_v2").or_else(|| d.get("tname")),
                        "duration": d.get("duration"),
                        "views": d.pointer("/stat/view"),
                        "danmaku": d.pointer("/stat/danmaku"),
                        "likes": d.pointer("/stat/like"),
                        "coins": d.pointer("/stat/coin"),
                        "favorites": d.pointer("/stat/favorite"),
                        "shares": d.pointer("/stat/share"),
                        "reply": d.pointer("/stat/reply"),
                        "pubdate": d.get("pubdate"),
                        "description": d.get("desc"),
                        "cover": d.get("pic"),
                        "url": d.get("bvid").and_then(|v| v.as_str()).map(|b| format!("https://www.bilibili.com/video/{b}")),
                    })
                } else {
                    json!({"error": "Video not found"})
                }
            },
        },

        BilibiliToolSpec {
            name: "bilibili_search",
            description: "搜索视频/用户",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "type": {"type": "string", "description": "搜索类型：video 或 user", "default": "video"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20},
                    "page": {"type": "integer", "description": "页码", "default": 1}
                },
                "required": ["query"]
            }),
            method: Method::GET,
            path: "/x/web-interface/wbi/search/type",
            build_params: |args| {
                let mut m = HashMap::new();
                let search_type = if args["type"].as_str() == Some("user") { "bili_user" } else { "video" };
                m.insert("search_type".into(), search_type.into());
                m.insert("keyword".into(), args["query"].as_str().unwrap_or("").into());
                m.insert("page".into(), args["page"].as_i64().unwrap_or(1).to_string());
                m.insert("page_size".into(), args["limit"].as_i64().unwrap_or(20).to_string());
                m
            },
            signed: true,
            parse_response: |resp| {
                let search_type = resp.get("data")
                    .and_then(|d| d.get("result"))
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter().take(20).map(|item| {
                            let is_user = item.get("mid").is_some();
                            if is_user {
                                json!({
                                    "type": "user",
                                    "name": item.get("uname").or_else(|| item.get("uname")),
                                    "mid": item.get("mid"),
                                    "sign": item.get("usign"),
                                    "fans": item.get("fans"),
                                    "videos": item.get("videos"),
                                })
                            } else {
                                parse_video(item)
                            }
                        }).collect::<Vec<_>>()
                    });
                json!(search_type.unwrap_or_default())
            },
        },

        BilibiliToolSpec {
            name: "bilibili_hot",
            description: "获取热门视频",
            schema: json!({
                "type": "object",
                "properties": { "limit": {"type": "integer", "description": "返回数量", "default": 20} }
            }),
            method: Method::GET,
            path: "/x/web-interface/popular",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("ps".into(), args["limit"].as_i64().unwrap_or(20).to_string());
                m.insert("pn".into(), "1".into());
                m
            },
            signed: false,
            parse_response: |resp| {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(20).map(|item| parse_video(item)).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_ranking",
            description: "获取排行榜",
            schema: json!({
                "type": "object",
                "properties": { "limit": {"type": "integer", "description": "返回数量", "default": 20} }
            }),
            method: Method::GET,
            path: "/x/web-interface/ranking/v2",
            build_params: |_| HashMap::new(),
            signed: false,
            parse_response: |resp| {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(20).map(|item| parse_video(item)).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_user_videos",
            description: "获取用户投稿视频",
            schema: json!({
                "type": "object",
                "properties": {
                    "uid": {"type": "string", "description": "用户 UID"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20},
                    "page": {"type": "integer", "description": "页码", "default": 1},
                    "order": {"type": "string", "description": "排序：pubdate/click/stow", "default": "pubdate"}
                },
                "required": ["uid"]
            }),
            method: Method::GET,
            path: "/x/space/wbi/arc/search",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("mid".into(), args["uid"].as_str().unwrap_or("").into());
                m.insert("pn".into(), args["page"].as_i64().unwrap_or(1).to_string());
                m.insert("ps".into(), args["limit"].as_i64().unwrap_or(20).to_string());
                m.insert("order".into(), args["order"].as_str().unwrap_or("pubdate").into());
                m
            },
            signed: true,
            parse_response: |resp| {
                let items = resp.pointer("/data/list/vlist")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(50).map(|item| {
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "bvid": item.get("bvid").and_then(|v| v.as_str()).unwrap_or(""),
                        "play": item.get("play").and_then(|v| v.as_i64()).unwrap_or(0),
                        "likes": item.get("like").and_then(|v| v.as_i64()).unwrap_or(0),
                        "created": item.get("created").and_then(|v| v.as_i64()),
                        "url": item.get("bvid").and_then(|v| v.as_str()).map(|b| format!("https://www.bilibili.com/video/{b}")).unwrap_or_default(),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_user_info",
            description: "获取用户信息",
            schema: json!({
                "type": "object",
                "properties": { "uid": {"type": "string", "description": "用户 UID"} },
                "required": ["uid"]
            }),
            method: Method::GET,
            path: "/x/space/wbi/acc/info",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("mid".into(), args["uid"].as_str().unwrap_or("").into());
                m
            },
            signed: true,
            parse_response: |resp| {
                let d = resp.pointer("/data");
                if let Some(d) = d {
                    json!({
                        "name": d.get("name"),
                        "mid": d.get("mid"),
                        "level": d.get("level"),
                        "sign": d.get("sign"),
                        "face": d.get("face"),
                        "fans": d.get("follower"),
                        "following": d.get("following"),
                        "coins": d.get("coins"),
                    })
                } else {
                    json!({"error": "User not found"})
                }
            },
        },

        BilibiliToolSpec {
            name: "bilibili_comments",
            description: "获取视频评论",
            schema: json!({
                "type": "object",
                "properties": {
                    "bvid": {"type": "string", "description": "视频 BV 号"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["bvid"]
            }),
            method: Method::GET,
            path: "/x/v2/reply/main",
            build_params: |args| {
                let mut m = HashMap::new();
                // Need to get oid (aid) from bvid first — use type=1 for video
                m.insert("type".into(), "1".into());
                m.insert("mode".into(), "3".into()); // 3=hot
                m.insert("ps".into(), args["limit"].as_i64().unwrap_or(20).to_string());
                // We need aid; bvid can be passed as oid for newer API
                m.insert("oid".into(), args["bvid"].as_str().unwrap_or("").into());
                m
            },
            signed: true,
            parse_response: |resp| {
                let replies = resp.pointer("/data/replies")
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default();
                replies.iter().take(20).map(|r| {
                    json!({
                        "author": r.pointer("/member/uname").and_then(|v| v.as_str()).unwrap_or(""),
                        "content": r.pointer("/content/message").and_then(|v| v.as_str()).unwrap_or(""),
                        "likes": r.get("like").and_then(|v| v.as_i64()).unwrap_or(0),
                        "replies": r.get("rcount").and_then(|v| v.as_i64()).unwrap_or(0),
                        "time": r.get("ctime").and_then(|v| v.as_i64()),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_feed",
            description: "获取动态",
            schema: json!({
                "type": "object",
                "properties": {
                    "uid": {"type": "string", "description": "用户 UID（可选）"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 15}
                }
            }),
            method: Method::GET,
            path: "/x/polymer/web-dynamic/v1/feed/all",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("page".into(), "1".into());
                // If uid provided, use space feed
                if let Some(uid) = args["uid"].as_str() {
                    m.insert("host_mid".into(), uid.into());
                }
                m
            },
            signed: false,
            parse_response: |resp| {
                let items = resp.pointer("/data/items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter()
                    .filter_map(|item| parse_dynamic_item(item))
                    .take(15)
                    .collect::<Vec<_>>()
                    .into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_following",
            description: "获取关注列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "uid": {"type": "string", "description": "用户 UID"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 50},
                    "page": {"type": "integer", "description": "页码", "default": 1}
                },
                "required": ["uid"]
            }),
            method: Method::GET,
            path: "/x/relation/followings",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("vmid".into(), args["uid"].as_str().unwrap_or("").into());
                m.insert("pn".into(), args["page"].as_i64().unwrap_or(1).to_string());
                m.insert("ps".into(), args["limit"].as_i64().unwrap_or(50).to_string());
                m.insert("order".into(), "desc".into());
                m
            },
            signed: false,
            parse_response: |resp| {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(50).map(|u| {
                    let attr = u.get("attribute").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "mid": u.get("mid"),
                        "name": u.get("uname").and_then(|v| v.as_str()).unwrap_or(""),
                        "sign": u.get("sign").and_then(|v| v.as_str()).unwrap_or(""),
                        "mutual": attr == 6,
                        "official": u.pointer("/official_verify/desc").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_favorite",
            description: "获取收藏夹",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20},
                    "page": {"type": "integer", "description": "页码", "default": 1}
                }
            }),
            method: Method::GET,
            path: "/x/v3/fav/resource/list",
            build_params: |args| {
                let mut m = HashMap::new();
                // Will be populated with media_id in tools.rs after listing folders
                m.insert("pn".into(), args["page"].as_i64().unwrap_or(1).to_string());
                m.insert("ps".into(), args["limit"].as_i64().unwrap_or(20).to_string());
                m.insert("platform".into(), "web".into());
                m
            },
            signed: true,
            parse_response: |resp| {
                let items = resp.pointer("/data/medias")
                    .and_then(|m| m.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(20).map(|item| {
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "author": item.get("upper").and_then(|u| u.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                        "play": item.pointer("/cnt_info/play").and_then(|v| v.as_i64()).unwrap_or(0),
                        "bvid": item.get("bvid").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_history",
            description: "获取观看历史",
            schema: json!({
                "type": "object",
                "properties": { "limit": {"type": "integer", "description": "返回数量", "default": 20} }
            }),
            method: Method::GET,
            path: "/x/web-interface/history/cursor",
            build_params: |_| {
                let mut m = HashMap::new();
                m.insert("ps".into(), "20".into());
                m
            },
            signed: false,
            parse_response: |resp| {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().take(20).map(|item| {
                    let progress = item.get("progress").and_then(|v| v.as_i64()).unwrap_or(0);
                    let duration = item.get("duration").and_then(|v| v.as_i64()).unwrap_or(1);
                    let pct = if duration > 0 { progress * 100 / duration } else { 0 };
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "author": item.get("author_name").and_then(|v| v.as_str()).unwrap_or(""),
                        "progress_pct": pct,
                        "bvid": item.pointer("/history/bvid").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        BilibiliToolSpec {
            name: "bilibili_subtitle",
            description: "获取视频字幕",
            schema: json!({
                "type": "object",
                "properties": {
                    "bvid": {"type": "string", "description": "视频 BV 号"},
                    "lang": {"type": "string", "description": "字幕语言", "default": "zh-CN"}
                },
                "required": ["bvid"]
            }),
            method: Method::GET,
            path: "/x/player/wbi/v2",
            build_params: |args| {
                let mut m = HashMap::new();
                m.insert("bvid".into(), args["bvid"].as_str().unwrap_or("").into());
                m.insert("cid".into(), "0".into()); // Will be filled in tools.rs
                m
            },
            signed: true,
            parse_response: |resp| {
                let subtitles = resp.pointer("/data/subtitle/subtitles")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut result: Vec<Value> = Vec::new();
                for sub in &subtitles {
                    result.push(json!({
                        "lang": sub.get("lan").and_then(|v| v.as_str()).unwrap_or(""),
                        "lang_name": sub.get("lan_doc").and_then(|v| v.as_str()).unwrap_or(""),
                        "url": sub.get("subtitle_url").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
                json!(result)
            },
        },

        BilibiliToolSpec {
            name: "bilibili_me",
            description: "获取当前账号信息",
            schema: json!({"type": "object", "properties": {}}),
            method: Method::GET,
            path: "/x/web-interface/nav",
            build_params: |_| HashMap::new(),
            signed: false,
            parse_response: |resp| {
                let d = resp.pointer("/data");
                if let Some(d) = d {
                    json!({
                        "name": d.get("uname"),
                        "mid": d.get("mid"),
                        "level": d.get("level_info").and_then(|l| l.get("current_level")),
                        "coins": d.get("money"),
                        "vip": d.get("vipStatus"),
                        "face": d.get("face"),
                    })
                } else {
                    json!({"error": "Not logged in"})
                }
            },
        },
    ]
}
