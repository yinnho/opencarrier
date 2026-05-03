//! VC (视频会议) tools — 3 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_search_meeting",
            description: "搜索飞书视频会议",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "page_size": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["query"]
            }),
            method: Method::POST,
            path: "open-apis/vc/v1/meetings/search",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_meeting",
            description: "获取飞书视频会议详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "会议ID"}
                },
                "required": ["meeting_id"]
            }),
            method: Method::GET,
            path: "open-apis/vc/v1/meetings/{meeting_id}",
            param_mapper: |args| {
                let meeting_id = args["meeting_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("meeting_id", meeting_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_get_recording",
            description: "获取飞书视频会议录制",
            schema: json!({
                "type": "object",
                "properties": {
                    "meeting_id": {"type": "string", "description": "会议ID"}
                },
                "required": ["meeting_id"]
            }),
            method: Method::GET,
            path: "open-apis/vc/v1/meetings/{meeting_id}/recording",
            param_mapper: |args| {
                let meeting_id = args["meeting_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("meeting_id", meeting_id)]),
                    query: None,
                    body: None,
                }
            },
        },
    ]
}
