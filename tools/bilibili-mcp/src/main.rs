//! bilibili-mcp — Bilibili MCP Server (multi-tenant, stateless)
//!
//! Each tool call carries `sessdata` + `bili_jct` + `dede_user_id` for cookie auth.
//! Public endpoints work without cookies; logged-in features require them.

mod api;

use std::collections::HashMap;

use anyhow::Result;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, transport::stdio as stdio_transport, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

// ================================================================== //
//  Parameter macro — injects cookie fields into every params struct   //
// ================================================================== //

macro_rules! define_params {
    ($name:ident { $($field:tt)* }) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        struct $name {
            #[schemars(description = "SESSDATA cookie（登录功能需要）")]
            sessdata: Option<String>,
            #[schemars(description = "bili_jct cookie（登录功能需要）")]
            bili_jct: Option<String>,
            #[schemars(description = "DedeUserID cookie（登录功能需要）")]
            dede_user_id: Option<String>,
            $($field)*
        }
    };
}

fn make_cookie(p: &impl CookieHolder) -> String {
    api::build_cookie(p.sessdata(), p.bili_jct(), p.dede_user_id())
}

trait CookieHolder {
    fn sessdata(&self) -> Option<&str>;
    fn bili_jct(&self) -> Option<&str>;
    fn dede_user_id(&self) -> Option<&str>;
}

macro_rules! impl_cookie {
    ($t:ty) => {
        impl CookieHolder for $t {
            fn sessdata(&self) -> Option<&str> { self.sessdata.as_deref() }
            fn bili_jct(&self) -> Option<&str> { self.bili_jct.as_deref() }
            fn dede_user_id(&self) -> Option<&str> { self.dede_user_id.as_deref() }
        }
    };
}

// ================================================================== //
//  Parameter structs                                                  //
// ================================================================== //

define_params!(VideoParams {
    #[schemars(description = "视频 BV 号")]
    bvid: String,
});
impl_cookie!(VideoParams);

define_params!(SearchParams {
    #[schemars(description = "搜索关键词")]
    query: String,
    #[schemars(description = "搜索类型：video 或 user（默认 video）")]
    r#type: Option<String>,
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
    #[schemars(description = "页码（默认 1）")]
    page: Option<i64>,
});
impl_cookie!(SearchParams);

define_params!(HotParams {
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
});
impl_cookie!(HotParams);

define_params!(RankingParams {
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
});
impl_cookie!(RankingParams);

define_params!(UserVideosParams {
    #[schemars(description = "用户 UID")]
    uid: String,
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
    #[schemars(description = "页码（默认 1）")]
    page: Option<i64>,
    #[schemars(description = "排序：pubdate/click/stow（默认 pubdate）")]
    order: Option<String>,
});
impl_cookie!(UserVideosParams);

define_params!(UserInfoParams {
    #[schemars(description = "用户 UID")]
    uid: String,
});
impl_cookie!(UserInfoParams);

define_params!(CommentsParams {
    #[schemars(description = "视频 BV 号")]
    bvid: String,
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
});
impl_cookie!(CommentsParams);

define_params!(FeedParams {
    #[schemars(description = "用户 UID（可选，不传则看自己的动态）")]
    uid: Option<String>,
    #[schemars(description = "返回数量（默认 15）")]
    limit: Option<i64>,
});
impl_cookie!(FeedParams);

define_params!(FollowingParams {
    #[schemars(description = "用户 UID")]
    uid: String,
    #[schemars(description = "返回数量（默认 50）")]
    limit: Option<i64>,
    #[schemars(description = "页码（默认 1）")]
    page: Option<i64>,
});
impl_cookie!(FollowingParams);

define_params!(FavoriteParams {
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
    #[schemars(description = "页码（默认 1）")]
    page: Option<i64>,
});
impl_cookie!(FavoriteParams);

define_params!(HistoryParams {
    #[schemars(description = "返回数量（默认 20）")]
    limit: Option<i64>,
});
impl_cookie!(HistoryParams);

define_params!(SubtitleParams {
    #[schemars(description = "视频 BV 号")]
    bvid: String,
});
impl_cookie!(SubtitleParams);

define_params!(MeParams {});
impl_cookie!(MeParams);

// ================================================================== //
//  Parsers                                                            //
// ================================================================== //

fn parse_video(item: &serde_json::Value) -> serde_json::Value {
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

fn parse_dynamic_item(item: &serde_json::Value) -> Option<serde_json::Value> {
    let author = item.pointer("/modules/module_author/name")
        .and_then(|v| v.as_str()).unwrap_or("");
    let id_str = item.get("id_str").and_then(|v| v.as_str()).unwrap_or("");
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

// ================================================================== //
//  MCP Server                                                         //
// ================================================================== //

#[derive(Clone)]
struct BilibiliServer;

#[tool_router(server_handler)]
impl BilibiliServer {
    // ---- Video ----

    #[tool(description = "获取视频信息")]
    async fn bilibili_video(&self, Parameters(params): Parameters<VideoParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("bvid".into(), params.bvid);
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/view", &m, false).await {
            Ok(resp) => {
                let d = resp.pointer("/data");
                let result = if let Some(d) = d {
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
                };
                api::truncate_result(result.to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Search ----

    #[tool(description = "搜索视频或用户")]
    async fn bilibili_search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let cookie = make_cookie(&params);
        let search_type = if params.r#type.as_deref() == Some("user") { "bili_user" } else { "video" };
        let mut m = HashMap::new();
        m.insert("search_type".into(), search_type.into());
        m.insert("keyword".into(), params.query);
        m.insert("page".into(), params.page.unwrap_or(1).to_string());
        m.insert("page_size".into(), params.limit.unwrap_or(20).to_string());

        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/wbi/search/type", &m, true).await {
            Ok(resp) => {
                let items = resp.get("data")
                    .and_then(|d| d.get("result"))
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter().take(20).map(|item| {
                            let is_user = item.get("mid").is_some();
                            if is_user {
                                json!({
                                    "type": "user",
                                    "name": item.get("uname"),
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
                api::truncate_result(json!(items.unwrap_or_default()).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Hot ----

    #[tool(description = "获取热门视频")]
    async fn bilibili_hot(&self, Parameters(params): Parameters<HotParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("ps".into(), params.limit.unwrap_or(20).to_string());
        m.insert("pn".into(), "1".into());
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/popular", &m, false).await {
            Ok(resp) => {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(20).map(|item| parse_video(item)).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Ranking ----

    #[tool(description = "获取排行榜")]
    async fn bilibili_ranking(&self, Parameters(params): Parameters<RankingParams>) -> String {
        let cookie = make_cookie(&params);
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/ranking/v2", &HashMap::new(), false).await {
            Ok(resp) => {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(params.limit.unwrap_or(20) as usize).map(|item| parse_video(item)).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- User Videos ----

    #[tool(description = "获取用户投稿视频")]
    async fn bilibili_user_videos(&self, Parameters(params): Parameters<UserVideosParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("mid".into(), params.uid);
        m.insert("pn".into(), params.page.unwrap_or(1).to_string());
        m.insert("ps".into(), params.limit.unwrap_or(20).to_string());
        m.insert("order".into(), params.order.unwrap_or_else(|| "pubdate".into()));
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/space/wbi/arc/search", &m, true).await {
            Ok(resp) => {
                let items = resp.pointer("/data/list/vlist")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(50).map(|item| {
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "bvid": item.get("bvid").and_then(|v| v.as_str()).unwrap_or(""),
                        "play": item.get("play").and_then(|v| v.as_i64()).unwrap_or(0),
                        "likes": item.get("like").and_then(|v| v.as_i64()).unwrap_or(0),
                        "created": item.get("created").and_then(|v| v.as_i64()),
                        "url": item.get("bvid").and_then(|v| v.as_str()).map(|b| format!("https://www.bilibili.com/video/{b}")).unwrap_or_default(),
                    })
                }).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- User Info ----

    #[tool(description = "获取用户信息")]
    async fn bilibili_user_info(&self, Parameters(params): Parameters<UserInfoParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("mid".into(), params.uid);
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/space/wbi/acc/info", &m, true).await {
            Ok(resp) => {
                let d = resp.pointer("/data");
                let result = if let Some(d) = d {
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
                };
                api::truncate_result(result.to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Comments ----

    #[tool(description = "获取视频评论")]
    async fn bilibili_comments(&self, Parameters(params): Parameters<CommentsParams>) -> String {
        let cookie = make_cookie(&params);

        // Need to resolve bvid -> aid first
        let mut nav = HashMap::new();
        nav.insert("bvid".into(), params.bvid.clone());
        let aid = match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/view", &nav, false).await {
            Ok(resp) => resp.pointer("/data/aid").and_then(|v| v.as_i64()),
            Err(_) => None,
        };

        let mut m = HashMap::new();
        m.insert("type".into(), "1".into());
        m.insert("mode".into(), "3".into());
        m.insert("ps".into(), params.limit.unwrap_or(20).to_string());
        if let Some(aid) = aid {
            m.insert("oid".into(), aid.to_string());
        } else {
            m.insert("oid".into(), params.bvid);
        }

        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/v2/reply/main", &m, true).await {
            Ok(resp) => {
                let replies = resp.pointer("/data/replies")
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = replies.iter().take(20).map(|r| {
                    json!({
                        "author": r.pointer("/member/uname").and_then(|v| v.as_str()).unwrap_or(""),
                        "content": r.pointer("/content/message").and_then(|v| v.as_str()).unwrap_or(""),
                        "likes": r.get("like").and_then(|v| v.as_i64()).unwrap_or(0),
                        "replies": r.get("rcount").and_then(|v| v.as_i64()).unwrap_or(0),
                        "time": r.get("ctime").and_then(|v| v.as_i64()),
                    })
                }).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Feed ----

    #[tool(description = "获取动态")]
    async fn bilibili_feed(&self, Parameters(params): Parameters<FeedParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("page".into(), "1".into());
        if let Some(uid) = params.uid {
            m.insert("host_mid".into(), uid);
        }
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/polymer/web-dynamic/v1/feed/all", &m, false).await {
            Ok(resp) => {
                let items = resp.pointer("/data/items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter()
                    .filter_map(|item| parse_dynamic_item(item))
                    .take(params.limit.unwrap_or(15) as usize)
                    .collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Following ----

    #[tool(description = "获取关注列表")]
    async fn bilibili_following(&self, Parameters(params): Parameters<FollowingParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("vmid".into(), params.uid);
        m.insert("pn".into(), params.page.unwrap_or(1).to_string());
        m.insert("ps".into(), params.limit.unwrap_or(50).to_string());
        m.insert("order".into(), "desc".into());
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/relation/followings", &m, false).await {
            Ok(resp) => {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(50).map(|u| {
                    let attr = u.get("attribute").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "mid": u.get("mid"),
                        "name": u.get("uname").and_then(|v| v.as_str()).unwrap_or(""),
                        "sign": u.get("sign").and_then(|v| v.as_str()).unwrap_or(""),
                        "mutual": attr == 6,
                        "official": u.pointer("/official_verify/desc").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Favorite ----

    #[tool(description = "获取收藏夹")]
    async fn bilibili_favorite(&self, Parameters(params): Parameters<FavoriteParams>) -> String {
        let cookie = make_cookie(&params);

        // Need to get media_id from folder list
        let uid = match api::get_self_uid(&cookie).await {
            Ok(u) => u,
            Err(e) => return json!({"error": format!("Not logged in: {e}")}).to_string(),
        };

        let mut folder_m = HashMap::new();
        folder_m.insert("up_mid".into(), uid.to_string());
        let media_id = match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/v3/fav/folder/created/list-all", &folder_m, true).await {
            Ok(resp) => resp.pointer("/data/list")
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("id").and_then(|v| v.as_i64())),
            Err(_) => None,
        };

        let mut m = HashMap::new();
        if let Some(mid) = media_id {
            m.insert("media_id".into(), mid.to_string());
        }
        m.insert("pn".into(), params.page.unwrap_or(1).to_string());
        m.insert("ps".into(), params.limit.unwrap_or(20).to_string());
        m.insert("platform".into(), "web".into());

        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/v3/fav/resource/list", &m, true).await {
            Ok(resp) => {
                let items = resp.pointer("/data/medias")
                    .and_then(|m| m.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(20).map(|item| {
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "author": item.get("upper").and_then(|u| u.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                        "play": item.pointer("/cnt_info/play").and_then(|v| v.as_i64()).unwrap_or(0),
                        "bvid": item.get("bvid").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- History ----

    #[tool(description = "获取观看历史")]
    async fn bilibili_history(&self, Parameters(params): Parameters<HistoryParams>) -> String {
        let cookie = make_cookie(&params);
        let mut m = HashMap::new();
        m.insert("ps".into(), params.limit.unwrap_or(20).to_string());
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/history/cursor", &m, false).await {
            Ok(resp) => {
                let items = resp.pointer("/data/list")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Vec<_> = items.iter().take(20).map(|item| {
                    let progress = item.get("progress").and_then(|v| v.as_i64()).unwrap_or(0);
                    let duration = item.get("duration").and_then(|v| v.as_i64()).unwrap_or(1);
                    let pct = if duration > 0 { progress * 100 / duration } else { 0 };
                    json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "author": item.get("author_name").and_then(|v| v.as_str()).unwrap_or(""),
                        "progress_pct": pct,
                        "bvid": item.pointer("/history/bvid").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                }).collect();
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Subtitle ----

    #[tool(description = "获取视频字幕")]
    async fn bilibili_subtitle(&self, Parameters(params): Parameters<SubtitleParams>) -> String {
        let cookie = make_cookie(&params);

        // Need to resolve bvid -> cid
        let mut nav = HashMap::new();
        nav.insert("bvid".into(), params.bvid.clone());
        let cid = match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/view", &nav, false).await {
            Ok(resp) => resp.pointer("/data/cid").and_then(|v| v.as_i64()),
            Err(_) => None,
        };

        let mut m = HashMap::new();
        m.insert("bvid".into(), params.bvid);
        if let Some(cid) = cid {
            m.insert("cid".into(), cid.to_string());
        } else {
            m.insert("cid".into(), "0".into());
        }

        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/player/wbi/v2", &m, true).await {
            Ok(resp) => {
                let subtitles = resp.pointer("/data/subtitle/subtitles")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut result: Vec<serde_json::Value> = Vec::new();
                for sub in &subtitles {
                    result.push(json!({
                        "lang": sub.get("lan").and_then(|v| v.as_str()).unwrap_or(""),
                        "lang_name": sub.get("lan_doc").and_then(|v| v.as_str()).unwrap_or(""),
                        "url": sub.get("subtitle_url").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
                api::truncate_result(json!(result).to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }

    // ---- Me ----

    #[tool(description = "获取当前账号信息")]
    async fn bilibili_me(&self, Parameters(params): Parameters<MeParams>) -> String {
        let cookie = make_cookie(&params);
        match api::bilibili_api(&cookie, reqwest::Method::GET, "/x/web-interface/nav", &HashMap::new(), false).await {
            Ok(resp) => {
                let d = resp.pointer("/data");
                let result = if let Some(d) = d {
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
                };
                api::truncate_result(result.to_string())
            }
            Err(e) => json!({"error": e}).to_string(),
        }
    }
}

// ================================================================== //
//  Entry point                                                        //
// ================================================================== //

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BILIBILI_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let server = BilibiliServer;
    tracing::info!("bilibili-mcp starting (stdio, multi-tenant)");
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;
    Ok(())
}
