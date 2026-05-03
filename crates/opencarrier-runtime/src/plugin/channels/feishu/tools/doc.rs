//! Doc (云文档) tools — 6 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_create_doc",
            description: "创建飞书文档",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "文档标题"},
                    "folder_token": {"type": "string", "description": "所在文件夹token"}
                },
                "required": ["title"]
            }),
            method: Method::POST,
            path: "open-apis/docx/v1/documents",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_doc",
            description: "获取飞书文档内容（块列表）",
            schema: json!({
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "文档ID"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 500},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["document_id"]
            }),
            method: Method::GET,
            path: "open-apis/docx/v1/documents/{document_id}/blocks",
            param_mapper: |args| {
                let document_id = args["document_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("document_id", document_id)]),
                    query: Some(json!(query)),
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_update_doc",
            description: "批量更新飞书文档块内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "文档ID"},
                    "requests": {"type": "array", "description": "更新操作列表", "items": {"type": "object"}}
                },
                "required": ["document_id", "requests"]
            }),
            method: Method::PATCH,
            path: "open-apis/docx/v1/documents/{document_id}/blocks/batch_update",
            param_mapper: |args| {
                let document_id = args["document_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "requests": args["requests"] });
                MappedParams {
                    path_params: HashMap::from([("document_id", document_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_search_docs",
            description: "搜索飞书文档",
            schema: json!({
                "type": "object",
                "properties": {
                    "search_key": {"type": "string", "description": "搜索关键词"},
                    "owner_ids": {"type": "array", "items": {"type": "string"}, "description": "文档所有者ID列表"},
                    "chat_ids": {"type": "array", "items": {"type": "string"}, "description": "关联群聊ID列表"},
                    "count": {"type": "integer", "description": "返回数量", "default": 20},
                    "offset": {"type": "integer", "description": "偏移量", "default": 0}
                },
                "required": ["search_key"]
            }),
            method: Method::POST,
            path: "open-apis/suite/docs/search",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_doc_raw",
            description: "获取旧版文档内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "文档ID"}
                },
                "required": ["document_id"]
            }),
            method: Method::GET,
            path: "open-apis/doc/v2/documents/{document_id}",
            param_mapper: |args| {
                let document_id = args["document_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("document_id", document_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_update_doc_raw",
            description: "更新旧版文档内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "document_id": {"type": "string", "description": "文档ID"},
                    "content": {"type": "string", "description": "文档内容"}
                },
                "required": ["document_id", "content"]
            }),
            method: Method::POST,
            path: "open-apis/doc/v2/documents/{document_id}",
            param_mapper: |args| {
                let document_id = args["document_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "content": args["content"] });
                MappedParams {
                    path_params: HashMap::from([("document_id", document_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
    ]
}
