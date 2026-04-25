//! Contact (通讯录) tools — 2 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_search_user",
            description: "搜索飞书用户",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词（姓名/邮箱等）"},
                    "page_size": {"type": "integer", "description": "返回数量", "default": 20}
                },
                "required": ["query"]
            }),
            method: Method::POST,
            path: "open-apis/search/v2/user",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_user",
            description: "获取飞书用户信息",
            schema: json!({
                "type": "object",
                "properties": {
                    "user_id": {"type": "string", "description": "用户ID"},
                    "user_id_type": {"type": "string", "description": "ID类型：open_id/user_id/union_id", "default": "open_id"}
                },
                "required": ["user_id"]
            }),
            method: Method::GET,
            path: "open-apis/contact/v3/users/{user_id}",
            param_mapper: |args| {
                let user_id = args["user_id"].as_str().unwrap_or("").to_string();
                let user_id_type = args["user_id_type"].as_str().unwrap_or("open_id").to_string();
                MappedParams {
                    path_params: HashMap::from([("user_id", user_id)]),
                    query: Some(json!({ "user_id_type": user_id_type })),
                    body: None,
                }
            },
        },
    ]
}
