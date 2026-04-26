//! Xiaohongshu tool specifications — 5 tools backed by Creator API.

use reqwest::Method;
use serde_json::{json, Value};

/// A Xiaohongshu tool specification.
pub struct XhsToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// API path template with optional {param} placeholders.
    pub path: &'static str,
    /// HTTP method (GET/POST).
    pub method: Method,
    /// Build query string from tool args (appended to path after param substitution).
    pub param_mapper: fn(&Value) -> String,
    /// Extract structured result from the API response.
    pub parse_response: fn(&Value) -> Value,
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

pub fn all_tools() -> Vec<XhsToolSpec> {
    vec![
        // 1. xhs_creator_notes — 创作者笔记列表
        XhsToolSpec {
            name: "xhs_creator_notes",
            description: "获取小红书创作者笔记列表",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                }
            }),
            path: "/api/galaxy/creator/datacenter/note/analyze/list",
            method: Method::GET,
            param_mapper: |args| {
                let limit = args["limit"].as_i64().unwrap_or(20);
                format!("type=0&page_size={limit}&page_num=1")
            },
            parse_response: |resp| {
                let notes = resp.pointer("/data/data")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter().map(|note| {
                            json!({
                                "id": note.get("id"),
                                "title": note.get("title"),
                                "post_time": note.get("post_time"),
                                "read_count": note.get("read_count"),
                                "like_count": note.get("like_count"),
                                "fav_count": note.get("fav_count"),
                                "comment_count": note.get("comment_count"),
                            })
                        }).collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                json!(notes)
            },
        },

        // 2. xhs_creator_note_detail — 单篇笔记详情
        XhsToolSpec {
            name: "xhs_creator_note_detail",
            description: "获取小红书单篇笔记详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "note_id": {"type": "string", "description": "笔记ID"}
                },
                "required": ["note_id"]
            }),
            path: "/api/galaxy/creator/datacenter/note/base",
            method: Method::GET,
            param_mapper: |args| {
                let note_id = args["note_id"].as_str().unwrap_or("");
                format!("note_id={note_id}")
            },
            parse_response: |resp| {
                resp.pointer("/data/data")
                    .cloned()
                    .unwrap_or_else(|| json!({"error": "Note not found"}))
            },
        },

        // 3. xhs_creator_profile — 创作者账号信息
        XhsToolSpec {
            name: "xhs_creator_profile",
            description: "获取小红书创作者账号信息",
            schema: json!({"type": "object", "properties": {}}),
            path: "/api/galaxy/creator/home/personal_info",
            method: Method::GET,
            param_mapper: |_| String::new(),
            parse_response: |resp| {
                let data = resp.pointer("/data/data");
                match data {
                    Some(d) => json!({
                        "name": d.get("name"),
                        "fans_count": d.get("fans_count"),
                        "follow_count": d.get("follow_count"),
                        "faved_count": d.get("faved_count"),
                        "personal_desc": d.get("personal_desc"),
                        "level": d.pointer("/grow_info/level"),
                    }),
                    None => json!({"error": "Profile not found"}),
                }
            },
        },

        // 4. xhs_creator_stats — 数据总览
        XhsToolSpec {
            name: "xhs_creator_stats",
            description: "获取小红书数据总览",
            schema: json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "description": "统计周期：seven(7天) 或 thirty(30天)",
                        "default": "seven",
                        "enum": ["seven", "thirty"]
                    }
                }
            }),
            path: "/api/galaxy/creator/data/note_detail_new",
            method: Method::GET,
            param_mapper: |_| String::new(),
            parse_response: |resp| {
                let data = resp.pointer("/data/data");
                match data {
                    Some(d) => {
                        // Extract both seven and thirty period data
                        let seven = d.get("seven").cloned().unwrap_or(json!(null));
                        let thirty = d.get("thirty").cloned().unwrap_or(json!(null));
                        json!({
                            "seven": seven,
                            "thirty": thirty,
                        })
                    },
                    None => json!({"error": "Stats not found"}),
                }
            },
        },

        // 5. xhs_creator_notes_summary — 笔记批量摘要
        XhsToolSpec {
            name: "xhs_creator_notes_summary",
            description: "获取小红书笔记批量摘要（笔记列表+详情汇总）",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 3}
                }
            }),
            path: "", // Composite tool — path is built dynamically
            method: Method::GET,
            param_mapper: |_| String::new(),
            parse_response: |_| json!(null), // Handled specially in tools.rs
        },
    ]
}
