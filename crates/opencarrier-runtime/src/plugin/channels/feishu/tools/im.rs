//! IM (即时通讯) tools — 8 tools.

use super::{body_only, query_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_send_message",
            description: "发送飞书消息（文本/图片/富文本等）",
            schema: json!({
                "type": "object",
                "properties": {
                    "receive_id": {"type": "string", "description": "接收者ID"},
                    "receive_id_type": {"type": "string", "description": "ID类型：open_id/user_id/union_id/chat_id/email", "default": "open_id"},
                    "msg_type": {"type": "string", "description": "消息类型：text/image/post/file等", "default": "text"},
                    "content": {"type": "string", "description": "消息内容（JSON字符串）"}
                },
                "required": ["receive_id", "content"]
            }),
            method: Method::POST,
            path: "open-apis/im/v1/messages",
            param_mapper: |args| {
                let receive_id_type = args["receive_id_type"].as_str().unwrap_or("open_id").to_string();
                let body = json!({
                    "receive_id": args["receive_id"],
                    "msg_type": args["msg_type"].as_str().unwrap_or("text"),
                    "content": args["content"],
                });
                MappedParams {
                    path_params: HashMap::new(),
                    query: Some(json!({ "receive_id_type": receive_id_type })),
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_reply_message",
            description: "回复飞书消息",
            schema: json!({
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "description": "被回复的消息ID"},
                    "msg_type": {"type": "string", "description": "消息类型", "default": "text"},
                    "content": {"type": "string", "description": "消息内容（JSON字符串）"}
                },
                "required": ["message_id", "content"]
            }),
            method: Method::POST,
            path: "open-apis/im/v1/messages/{message_id}/reply",
            param_mapper: |args| {
                let message_id = args["message_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "content": args["content"],
                    "msg_type": args["msg_type"].as_str().unwrap_or("text"),
                });
                MappedParams {
                    path_params: HashMap::from([("message_id", message_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_search_messages",
            description: "搜索飞书消息",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "chat_id": {"type": "string", "description": "限定群聊ID"},
                    "message_type": {"type": "string", "description": "消息类型过滤"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20}
                },
                "required": ["query"]
            }),
            method: Method::POST,
            path: "open-apis/im/v1/messages/search",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_list_messages",
            description: "获取聊天会话消息记录",
            schema: json!({
                "type": "object",
                "properties": {
                    "container_id": {"type": "string", "description": "群聊或单聊ID"},
                    "container_id_type": {"type": "string", "description": "容器类型", "default": "chat"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["container_id"]
            }),
            method: Method::GET,
            path: "open-apis/im/v1/messages",
            param_mapper: query_only,
        },
        ToolSpec {
            name: "feishu_create_chat",
            description: "创建飞书群聊",
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "群名"},
                    "description": {"type": "string", "description": "群描述"},
                    "chat_mode": {"type": "string", "description": "群类型", "default": "group"},
                    "chat_type": {"type": "string", "description": "群可见性", "default": "private"},
                    "user_id_list": {"type": "array", "items": {"type": "string"}, "description": "初始成员列表"}
                },
                "required": ["name"]
            }),
            method: Method::POST,
            path: "open-apis/im/v1/chats",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_update_chat",
            description: "更新飞书群聊信息",
            schema: json!({
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string", "description": "群聊ID"},
                    "name": {"type": "string", "description": "新群名"},
                    "description": {"type": "string", "description": "新描述"}
                },
                "required": ["chat_id"]
            }),
            method: Method::PUT,
            path: "open-apis/im/v1/chats/{chat_id}",
            param_mapper: |args| {
                let chat_id = args["chat_id"].as_str().unwrap_or("").to_string();
                let mut body = serde_json::Map::new();
                if let Some(v) = args.get("name") { body.insert("name".into(), v.clone()); }
                if let Some(v) = args.get("description") { body.insert("description".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("chat_id", chat_id)]),
                    query: None,
                    body: Some(Value::Object(body)),
                }
            },
        },
        ToolSpec {
            name: "feishu_list_chats",
            description: "列出用户所在的群聊",
            schema: json!({
                "type": "object",
                "properties": {
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                }
            }),
            method: Method::GET,
            path: "open-apis/im/v1/chats",
            param_mapper: query_only,
        },
        ToolSpec {
            name: "feishu_download_resource",
            description: "下载飞书消息中的文件资源",
            schema: json!({
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "description": "消息ID"},
                    "file_key": {"type": "string", "description": "文件key"},
                    "type": {"type": "string", "description": "资源类型：file/image/video", "default": "file"}
                },
                "required": ["message_id", "file_key"]
            }),
            method: Method::GET,
            path: "open-apis/im/v1/messages/{message_id}/resources/{file_key}",
            param_mapper: |args| {
                let message_id = args["message_id"].as_str().unwrap_or("").to_string();
                let file_key = args["file_key"].as_str().unwrap_or("").to_string();
                let resource_type = args["type"].as_str().unwrap_or("file").to_string();
                MappedParams {
                    path_params: HashMap::from([
                        ("message_id", message_id),
                        ("file_key", file_key),
                    ]),
                    query: Some(json!({ "type": resource_type })),
                    body: None,
                }
            },
        },
    ]
}
