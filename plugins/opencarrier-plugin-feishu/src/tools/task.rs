//! Task (任务) tools — 6 tools.

use super::{body_only, query_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_create_task",
            description: "创建飞书任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "任务标题"},
                    "description": {"type": "string", "description": "任务描述"},
                    "due_date": {"type": "object", "description": "截止时间"},
                    "assignees": {"type": "array", "items": {"type": "object"}, "description": "负责人列表"}
                },
                "required": ["summary"]
            }),
            method: Method::POST,
            path: "open-apis/task/v2/tasks",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_get_task",
            description: "获取飞书任务详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "任务ID"}
                },
                "required": ["task_id"]
            }),
            method: Method::GET,
            path: "open-apis/task/v2/tasks/{task_id}",
            param_mapper: |args| {
                let task_id = args["task_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("task_id", task_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_update_task",
            description: "更新飞书任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "任务ID"},
                    "summary": {"type": "string", "description": "任务标题"},
                    "description": {"type": "string", "description": "任务描述"},
                    "due_date": {"type": "object", "description": "截止时间"}
                },
                "required": ["task_id"]
            }),
            method: Method::PATCH,
            path: "open-apis/task/v2/tasks/{task_id}",
            param_mapper: |args| {
                let task_id = args["task_id"].as_str().unwrap_or("").to_string();
                let mut body = serde_json::Map::new();
                if let Some(v) = args.get("summary") { body.insert("summary".into(), v.clone()); }
                if let Some(v) = args.get("description") { body.insert("description".into(), v.clone()); }
                if let Some(v) = args.get("due_date") { body.insert("due_date".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("task_id", task_id)]),
                    query: None,
                    body: if body.is_empty() { None } else { Some(json!(body)) },
                }
            },
        },
        ToolSpec {
            name: "feishu_complete_task",
            description: "完成飞书任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "任务ID"}
                },
                "required": ["task_id"]
            }),
            method: Method::POST,
            path: "open-apis/task/v2/tasks/{task_id}/complete",
            param_mapper: |args| {
                let task_id = args["task_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("task_id", task_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_list_my_tasks",
            description: "列出我的飞书任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"},
                    "start_time": {"type": "string", "description": "开始时间"},
                    "end_time": {"type": "string", "description": "结束时间"}
                }
            }),
            method: Method::GET,
            path: "open-apis/task/v2/tasks",
            param_mapper: query_only,
        },
        ToolSpec {
            name: "feishu_search_tasks",
            description: "搜索飞书任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["query"]
            }),
            method: Method::POST,
            path: "open-apis/task/v2/tasks/search",
            param_mapper: body_only,
        },
    ]
}
