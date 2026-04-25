//! Minutes (会议纪要) tools — 2 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_search_minutes",
            description: "搜索飞书会议纪要",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "page_size": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["query"]
            }),
            method: Method::POST,
            path: "open-apis/minutes/v1/minutes/search",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_minutes",
            description: "获取飞书会议纪要详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "minutes_id": {"type": "string", "description": "纪要ID"}
                },
                "required": ["minutes_id"]
            }),
            method: Method::GET,
            path: "open-apis/minutes/v1/minutes/{minutes_id}",
            param_mapper: |args| {
                let minutes_id = args["minutes_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("minutes_id", minutes_id)]),
                    query: None,
                    body: None,
                }
            },
        },
    ]
}
