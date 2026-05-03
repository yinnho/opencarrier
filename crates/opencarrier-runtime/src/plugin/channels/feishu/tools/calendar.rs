//! Calendar (日历) tools — 6 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_create_event",
            description: "创建飞书日历日程",
            schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "日历ID"},
                    "summary": {"type": "string", "description": "日程标题"},
                    "description": {"type": "string", "description": "日程描述"},
                    "start_time": {"type": "object", "description": "开始时间 {timestamp, timezone}"},
                    "end_time": {"type": "object", "description": "结束时间 {timestamp, timezone}"},
                    "attendees": {"type": "array", "items": {"type": "object"}, "description": "参与人列表"},
                    "visibility": {"type": "string", "description": "可见性：default/public/private", "default": "default"},
                    "reminders": {"type": "array", "items": {"type": "object"}, "description": "提醒列表"}
                },
                "required": ["calendar_id", "summary", "start_time", "end_time"]
            }),
            method: Method::POST,
            path: "open-apis/calendar/v4/calendars/{calendar_id}/events",
            param_mapper: |args| {
                let calendar_id = args["calendar_id"].as_str().unwrap_or("").to_string();
                let mut body = serde_json::Map::new();
                if let Some(v) = args.get("summary") { body.insert("summary".into(), v.clone()); }
                if let Some(v) = args.get("description") { body.insert("description".into(), v.clone()); }
                if let Some(v) = args.get("start_time") { body.insert("start_time".into(), v.clone()); }
                if let Some(v) = args.get("end_time") { body.insert("end_time".into(), v.clone()); }
                if let Some(v) = args.get("attendees") { body.insert("attendees".into(), v.clone()); }
                if let Some(v) = args.get("visibility") { body.insert("visibility".into(), v.clone()); }
                if let Some(v) = args.get("reminders") { body.insert("reminders".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("calendar_id", calendar_id)]),
                    query: None,
                    body: Some(json!(body)),
                }
            },
        },
        ToolSpec {
            name: "feishu_get_event",
            description: "获取飞书日历日程详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "日历ID"},
                    "event_id": {"type": "string", "description": "日程ID"}
                },
                "required": ["calendar_id", "event_id"]
            }),
            method: Method::GET,
            path: "open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}",
            param_mapper: |args| {
                let calendar_id = args["calendar_id"].as_str().unwrap_or("").to_string();
                let event_id = args["event_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("calendar_id", calendar_id), ("event_id", event_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_list_events",
            description: "列出飞书日历日程",
            schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "日历ID"},
                    "start_time": {"type": "string", "description": "开始时间（Unix时间戳）"},
                    "end_time": {"type": "string", "description": "结束时间（Unix时间戳）"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 50},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["calendar_id", "start_time", "end_time"]
            }),
            method: Method::GET,
            path: "open-apis/calendar/v4/calendars/{calendar_id}/events",
            param_mapper: |args| {
                let calendar_id = args["calendar_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("start_time") { query.insert("start_time".into(), v.clone()); }
                if let Some(v) = args.get("end_time") { query.insert("end_time".into(), v.clone()); }
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("calendar_id", calendar_id)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_update_event",
            description: "更新飞书日历日程",
            schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "日历ID"},
                    "event_id": {"type": "string", "description": "日程ID"},
                    "summary": {"type": "string", "description": "日程标题"},
                    "description": {"type": "string", "description": "日程描述"},
                    "start_time": {"type": "object", "description": "开始时间"},
                    "end_time": {"type": "object", "description": "结束时间"}
                },
                "required": ["calendar_id", "event_id"]
            }),
            method: Method::PATCH,
            path: "open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}",
            param_mapper: |args| {
                let calendar_id = args["calendar_id"].as_str().unwrap_or("").to_string();
                let event_id = args["event_id"].as_str().unwrap_or("").to_string();
                let mut body = serde_json::Map::new();
                if let Some(v) = args.get("summary") { body.insert("summary".into(), v.clone()); }
                if let Some(v) = args.get("description") { body.insert("description".into(), v.clone()); }
                if let Some(v) = args.get("start_time") { body.insert("start_time".into(), v.clone()); }
                if let Some(v) = args.get("end_time") { body.insert("end_time".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("calendar_id", calendar_id), ("event_id", event_id)]),
                    query: None,
                    body: if body.is_empty() { None } else { Some(json!(body)) },
                }
            },
        },
        ToolSpec {
            name: "feishu_freebusy",
            description: "查询飞书用户忙闲状态",
            schema: json!({
                "type": "object",
                "properties": {
                    "time_min": {"type": "string", "description": "查询开始时间（ISO 8601）"},
                    "time_max": {"type": "string", "description": "查询结束时间（ISO 8601）"},
                    "user_id": {"type": "object", "description": "查询用户ID列表"}
                },
                "required": ["time_min", "time_max", "user_id"]
            }),
            method: Method::POST,
            path: "open-apis/calendar/v4/freebusy/list",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_search_event",
            description: "搜索飞书日历日程",
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
            path: "open-apis/calendar/v4/events/search",
            param_mapper: body_only,
        },
    ]
}
