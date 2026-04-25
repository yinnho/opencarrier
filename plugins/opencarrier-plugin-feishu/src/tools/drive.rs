//! Drive (云空间) tools — 6 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_upload_file",
            description: "上传文件到飞书云空间",
            schema: json!({
                "type": "object",
                "properties": {
                    "parent_node": {"type": "string", "description": "父文件夹token"},
                    "file_name": {"type": "string", "description": "文件名"},
                    "file_type": {"type": "string", "description": "文件类型：doc/sheet/bitable等"},
                    "title": {"type": "string", "description": "文档标题"}
                },
                "required": ["parent_node"]
            }),
            method: Method::POST,
            path: "open-apis/drive/v1/files/upload_all",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_download_file",
            description: "下载飞书云空间文件",
            schema: json!({
                "type": "object",
                "properties": {
                    "file_token": {"type": "string", "description": "文件token"},
                    "file_type": {"type": "string", "description": "文件类型", "default": "file"}
                },
                "required": ["file_token"]
            }),
            method: Method::GET,
            path: "open-apis/drive/v1/files/{file_token}",
            param_mapper: |args| {
                let file_token = args["file_token"].as_str().unwrap_or("").to_string();
                let file_type = args["file_type"].as_str().unwrap_or("file").to_string();
                MappedParams {
                    path_params: HashMap::from([("file_token", file_token)]),
                    query: Some(json!({ "file_type": file_type })),
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_create_folder",
            description: "在飞书云空间创建文件夹",
            schema: json!({
                "type": "object",
                "properties": {
                    "parent_token": {"type": "string", "description": "父文件夹token"},
                    "name": {"type": "string", "description": "文件夹名称"},
                    "folder_type": {"type": "string", "description": "文件夹类型", "default": "doc"}
                },
                "required": ["parent_token", "name"]
            }),
            method: Method::POST,
            path: "open-apis/drive/v1/files/create_folder",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_search_drive",
            description: "搜索飞书云空间文件",
            schema: json!({
                "type": "object",
                "properties": {
                    "search_key": {"type": "string", "description": "搜索关键词"},
                    "owner_ids": {"type": "array", "items": {"type": "string"}, "description": "所有者ID列表"},
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
            name: "feishu_move_file",
            description: "移动飞书云空间文件",
            schema: json!({
                "type": "object",
                "properties": {
                    "file_token": {"type": "string", "description": "文件token"},
                    "file_type": {"type": "string", "description": "文件类型", "default": "file"},
                    "folder_token": {"type": "string", "description": "目标文件夹token"}
                },
                "required": ["file_token", "folder_token"]
            }),
            method: Method::POST,
            path: "open-apis/drive/v1/files/{file_token}/move",
            param_mapper: |args| {
                let file_token = args["file_token"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "file_type": args["file_type"].as_str().unwrap_or("file"),
                    "folder_token": args["folder_token"],
                });
                MappedParams {
                    path_params: HashMap::from([("file_token", file_token)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_delete_file",
            description: "删除飞书云空间文件",
            schema: json!({
                "type": "object",
                "properties": {
                    "file_token": {"type": "string", "description": "文件token"},
                    "file_type": {"type": "string", "description": "文件类型", "default": "file"}
                },
                "required": ["file_token"]
            }),
            method: Method::DELETE,
            path: "open-apis/drive/v1/files/{file_token}",
            param_mapper: |args| {
                let file_token = args["file_token"].as_str().unwrap_or("").to_string();
                let file_type = args["file_type"].as_str().unwrap_or("file").to_string();
                MappedParams {
                    path_params: HashMap::from([("file_token", file_token)]),
                    query: Some(json!({ "file_type": file_type })),
                    body: None,
                }
            },
        },
    ]
}
