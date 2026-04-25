//! Sheets (电子表格) tools — 6 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_create_sheet",
            description: "创建飞书电子表格",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "表格标题"},
                    "folder_token": {"type": "string", "description": "所在文件夹token"}
                },
                "required": ["title"]
            }),
            method: Method::POST,
            path: "open-apis/sheets/v3/spreadsheets",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_read_sheet",
            description: "读取电子表格数据",
            schema: json!({
                "type": "object",
                "properties": {
                    "spreadsheet_token": {"type": "string", "description": "表格token"},
                    "range": {"type": "string", "description": "读取范围（如 Sheet1!A1:B10）"}
                },
                "required": ["spreadsheet_token", "range"]
            }),
            method: Method::GET,
            path: "open-apis/sheets/v2/spreadsheets/{spreadsheet_token}/values/{range}",
            param_mapper: |args| {
                let spreadsheet_token = args["spreadsheet_token"].as_str().unwrap_or("").to_string();
                let range = args["range"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([
                        ("spreadsheet_token", spreadsheet_token),
                        ("range", range),
                    ]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_write_sheet",
            description: "写入电子表格数据",
            schema: json!({
                "type": "object",
                "properties": {
                    "spreadsheet_token": {"type": "string", "description": "表格token"},
                    "range": {"type": "string", "description": "写入范围"},
                    "values": {"type": "array", "items": {"type": "array"}, "description": "二维数组数据"}
                },
                "required": ["spreadsheet_token", "range", "values"]
            }),
            method: Method::PUT,
            path: "open-apis/sheets/v2/spreadsheets/{spreadsheet_token}/values",
            param_mapper: |args| {
                let spreadsheet_token = args["spreadsheet_token"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "valueRange": {
                        "range": args["range"],
                        "values": args["values"]
                    }
                });
                MappedParams {
                    path_params: HashMap::from([("spreadsheet_token", spreadsheet_token)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_append_sheet",
            description: "追加数据到电子表格",
            schema: json!({
                "type": "object",
                "properties": {
                    "spreadsheet_token": {"type": "string", "description": "表格token"},
                    "range": {"type": "string", "description": "追加起始范围"},
                    "values": {"type": "array", "items": {"type": "array"}, "description": "二维数组数据"}
                },
                "required": ["spreadsheet_token", "range", "values"]
            }),
            method: Method::POST,
            path: "open-apis/sheets/v2/spreadsheets/{spreadsheet_token}/values_append",
            param_mapper: |args| {
                let spreadsheet_token = args["spreadsheet_token"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "valueRange": {
                        "range": args["range"],
                        "values": args["values"]
                    }
                });
                MappedParams {
                    path_params: HashMap::from([("spreadsheet_token", spreadsheet_token)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_find_in_sheet",
            description: "在电子表格中查找数据",
            schema: json!({
                "type": "object",
                "properties": {
                    "spreadsheet_token": {"type": "string", "description": "表格token"},
                    "sheet_id": {"type": "string", "description": "子表ID"},
                    "find_condition": {"type": "object", "description": "查找条件"}
                },
                "required": ["spreadsheet_token", "sheet_id", "find_condition"]
            }),
            method: Method::POST,
            path: "open-apis/sheets/v2/spreadsheets/{spreadsheet_token}/sheets/{sheet_id}/find",
            param_mapper: |args| {
                let spreadsheet_token = args["spreadsheet_token"].as_str().unwrap_or("").to_string();
                let sheet_id = args["sheet_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "find_condition": args["find_condition"] });
                MappedParams {
                    path_params: HashMap::from([
                        ("spreadsheet_token", spreadsheet_token),
                        ("sheet_id", sheet_id),
                    ]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_export_sheet",
            description: "导出电子表格",
            schema: json!({
                "type": "object",
                "properties": {
                    "spreadsheet_token": {"type": "string", "description": "表格token"},
                    "sheet_id": {"type": "string", "description": "子表ID"},
                    "export_format": {"type": "string", "description": "导出格式：xlsx/csv", "default": "xlsx"}
                },
                "required": ["spreadsheet_token"]
            }),
            method: Method::POST,
            path: "open-apis/sheets/v2/spreadsheets/{spreadsheet_token}/export",
            param_mapper: |args| {
                let spreadsheet_token = args["spreadsheet_token"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "sheet_id": args["sheet_id"],
                    "export_format": args["export_format"].as_str().unwrap_or("xlsx"),
                });
                MappedParams {
                    path_params: HashMap::from([("spreadsheet_token", spreadsheet_token)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
    ]
}
