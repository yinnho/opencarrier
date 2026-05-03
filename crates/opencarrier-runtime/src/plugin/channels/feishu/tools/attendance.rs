//! Attendance (考勤) tools — 1 tool.

use super::{body_only, ToolSpec};
use reqwest::Method;
use serde_json::json;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_get_attendance",
            description: "查询飞书考勤记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "user_ids": {"type": "array", "items": {"type": "string"}, "description": "用户ID列表"},
                    "start_time": {"type": "string", "description": "查询开始时间（Unix时间戳）"},
                    "end_time": {"type": "string", "description": "查询结束时间（Unix时间戳）"}
                },
                "required": ["user_ids", "start_time", "end_time"]
            }),
            method: Method::POST,
            path: "open-apis/attendance/v1/userTasks/query",
            param_mapper: body_only,
        },
    ]
}
