//! Wiki (知识库) tools — 3 tools.

use super::{body_only, query_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_list_spaces",
            description: "列出飞书知识库空间",
            schema: json!({
                "type": "object",
                "properties": {
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                }
            }),
            method: Method::GET,
            path: "open-apis/wiki/v2/spaces",
            param_mapper: query_only,
        },
        ToolSpec {
            name: "feishu_create_node",
            description: "在飞书知识库中创建节点",
            schema: json!({
                "type": "object",
                "properties": {
                    "space_id": {"type": "string", "description": "知识空间ID"},
                    "node_type": {"type": "string", "description": "节点类型：doc/docx/wiki/sheet/bitable", "default": "docx"},
                    "title": {"type": "string", "description": "节点标题"},
                    "parent_node_token": {"type": "string", "description": "父节点token"}
                },
                "required": ["space_id", "title"]
            }),
            method: Method::POST,
            path: "open-apis/wiki/v2/spaces/{space_id}/nodes",
            param_mapper: |args| {
                let space_id = args["space_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "node_type": args["node_type"].as_str().unwrap_or("docx"),
                    "title": args["title"],
                });
                MappedParams {
                    path_params: HashMap::from([("space_id", space_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_get_node",
            description: "获取飞书知识库节点信息",
            schema: json!({
                "type": "object",
                "properties": {
                    "token": {"type": "string", "description": "节点token"}
                },
                "required": ["token"]
            }),
            method: Method::GET,
            path: "open-apis/wiki/v2/spaces/get_node",
            param_mapper: |args| {
                let token = args["token"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::new(),
                    query: Some(json!({ "token": token })),
                    body: None,
                }
            },
        },
    ]
}
