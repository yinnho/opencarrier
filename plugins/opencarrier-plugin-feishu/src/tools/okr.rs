//! OKR tools — 2 tools.

use super::{MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_list_okr_cycles",
            description: "列出飞书OKR周期",
            schema: json!({
                "type": "object",
                "properties": {
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                }
            }),
            method: Method::GET,
            path: "open-apis/okr/v1/cycles",
            param_mapper: |args| {
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::new(),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_get_okr_detail",
            description: "获取飞书OKR周期目标和关键结果",
            schema: json!({
                "type": "object",
                "properties": {
                    "cycle_id": {"type": "string", "description": "OKR周期ID"},
                    "user_id": {"type": "string", "description": "用户ID"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20}
                },
                "required": ["cycle_id"]
            }),
            method: Method::GET,
            path: "open-apis/okr/v1/cycles/{cycle_id}/objectives",
            param_mapper: |args| {
                let cycle_id = args["cycle_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("user_id") { query.insert("user_id".into(), v.clone()); }
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("cycle_id", cycle_id)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
    ]
}
