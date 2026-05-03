//! Base / Bitable (多维表格) tools — 8 tools.

use super::{MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_list_tables",
            description: "列出多维表格的数据表",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["app_token"]
            }),
            method: Method::GET,
            path: "open-apis/bitable/v1/apps/{app_token}/tables",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_create_table",
            description: "在多维表格中创建数据表",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table": {"type": "object", "description": "数据表定义（含name和fields）"}
                },
                "required": ["app_token", "table"]
            }),
            method: Method::POST,
            path: "open-apis/bitable/v1/apps/{app_token}/tables",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let body = json!({ "table": args["table"] });
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_list_fields",
            description: "列出多维表格字段",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20}
                },
                "required": ["app_token", "table_id"]
            }),
            method: Method::GET,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token), ("table_id", table_id)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_create_field",
            description: "在多维表格中创建字段",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "field_name": {"type": "string", "description": "字段名"},
                    "type": {"type": "integer", "description": "字段类型（1=文本,2=数字,3=单选,4=多选,5=日期,7=复选框,11=人员,13=电话,15=URL,17=公式,18=关联,21=单向关联,22=双向关联,23=位置,1001=创建人,1002=修改人,1003=创建时间,1004=修改时间）"}
                },
                "required": ["app_token", "table_id", "field_name", "type"]
            }),
            method: Method::POST,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "field_name": args["field_name"],
                    "type": args["type"],
                });
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token), ("table_id", table_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_list_records",
            description: "列出多维表格记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"},
                    "filter": {"type": "string", "description": "过滤条件"},
                    "sort": {"type": "string", "description": "排序条件"}
                },
                "required": ["app_token", "table_id"]
            }),
            method: Method::GET,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                if let Some(v) = args.get("filter") { query.insert("filter".into(), v.clone()); }
                if let Some(v) = args.get("sort") { query.insert("sort".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token), ("table_id", table_id)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_create_record",
            description: "在多维表格中创建记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "fields": {"type": "object", "description": "字段值映射"}
                },
                "required": ["app_token", "table_id", "fields"]
            }),
            method: Method::POST,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "fields": args["fields"] });
                MappedParams {
                    path_params: HashMap::from([("app_token", app_token), ("table_id", table_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_update_record",
            description: "更新多维表格记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "record_id": {"type": "string", "description": "记录ID"},
                    "fields": {"type": "object", "description": "要更新的字段值"}
                },
                "required": ["app_token", "table_id", "record_id", "fields"]
            }),
            method: Method::PUT,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let record_id = args["record_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "fields": args["fields"] });
                MappedParams {
                    path_params: HashMap::from([
                        ("app_token", app_token),
                        ("table_id", table_id),
                        ("record_id", record_id),
                    ]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_delete_record",
            description: "删除多维表格记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "app_token": {"type": "string", "description": "多维表格app_token"},
                    "table_id": {"type": "string", "description": "数据表ID"},
                    "record_id": {"type": "string", "description": "记录ID"}
                },
                "required": ["app_token", "table_id", "record_id"]
            }),
            method: Method::DELETE,
            path: "open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}",
            param_mapper: |args| {
                let app_token = args["app_token"].as_str().unwrap_or("").to_string();
                let table_id = args["table_id"].as_str().unwrap_or("").to_string();
                let record_id = args["record_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([
                        ("app_token", app_token),
                        ("table_id", table_id),
                        ("record_id", record_id),
                    ]),
                    query: None,
                    body: None,
                }
            },
        },
    ]
}
