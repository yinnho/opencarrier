//! Approval (审批) tools — 4 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_get_approval",
            description: "获取飞书审批实例详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "instance_id": {"type": "string", "description": "审批实例ID"},
                    "user_id_type": {"type": "string", "description": "用户ID类型", "default": "open_id"}
                },
                "required": ["instance_id"]
            }),
            method: Method::GET,
            path: "open-apis/approval/v4/instances/{instance_id}",
            param_mapper: |args| {
                let instance_id = args["instance_id"].as_str().unwrap_or("").to_string();
                let user_id_type = args["user_id_type"].as_str().unwrap_or("open_id").to_string();
                MappedParams {
                    path_params: HashMap::from([("instance_id", instance_id)]),
                    query: Some(json!({ "user_id_type": user_id_type })),
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_approve_task",
            description: "同意飞书审批任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "审批任务ID"},
                    "user_id": {"type": "string", "description": "审批人ID"},
                    "comment": {"type": "string", "description": "审批意见"}
                },
                "required": ["task_id", "user_id"]
            }),
            method: Method::POST,
            path: "open-apis/approval/v4/tasks/approve",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_reject_task",
            description: "拒绝飞书审批任务",
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "审批任务ID"},
                    "user_id": {"type": "string", "description": "审批人ID"},
                    "comment": {"type": "string", "description": "拒绝理由"}
                },
                "required": ["task_id", "user_id"]
            }),
            method: Method::POST,
            path: "open-apis/approval/v4/tasks/reject",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_list_approvals",
            description: "列出飞书审批实例",
            schema: json!({
                "type": "object",
                "properties": {
                    "approval_code": {"type": "string", "description": "审批定义code"},
                    "start_time": {"type": "string", "description": "开始时间（Unix时间戳）"},
                    "end_time": {"type": "string", "description": "结束时间（Unix时间戳）"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["approval_code", "start_time", "end_time"]
            }),
            method: Method::POST,
            path: "open-apis/approval/v4/instance/list",
            param_mapper: body_only,
        },
    ]
}
