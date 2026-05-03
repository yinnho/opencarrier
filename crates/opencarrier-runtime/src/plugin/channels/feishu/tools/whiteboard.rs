//! Whiteboard (白板) tools — 2 tools.

use super::{MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_query_whiteboard",
            description: "查询飞书白板信息",
            schema: json!({
                "type": "object",
                "properties": {
                    "whiteboard_id": {"type": "string", "description": "白板ID"}
                },
                "required": ["whiteboard_id"]
            }),
            method: Method::GET,
            path: "open-apis-whiteboard/v1/whiteboards/{whiteboard_id}",
            param_mapper: |args| {
                let whiteboard_id = args["whiteboard_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("whiteboard_id", whiteboard_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_update_whiteboard",
            description: "更新飞书白板",
            schema: json!({
                "type": "object",
                "properties": {
                    "whiteboard_id": {"type": "string", "description": "白板ID"},
                    "title": {"type": "string", "description": "白板标题"}
                },
                "required": ["whiteboard_id"]
            }),
            method: Method::PATCH,
            path: "open-apis-whiteboard/v1/whiteboards/{whiteboard_id}",
            param_mapper: |args| {
                let whiteboard_id = args["whiteboard_id"].as_str().unwrap_or("").to_string();
                let mut body = serde_json::Map::new();
                if let Some(v) = args.get("title") { body.insert("title".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("whiteboard_id", whiteboard_id)]),
                    query: None,
                    body: if body.is_empty() { None } else { Some(json!(body)) },
                }
            },
        },
    ]
}
