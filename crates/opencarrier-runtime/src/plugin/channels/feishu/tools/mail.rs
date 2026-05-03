//! Mail (邮箱) tools — 6 tools.

use super::{MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_send_mail",
            description: "发送飞书邮件",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID（user_id或mail_alias_id）"},
                    "subject": {"type": "string", "description": "邮件主题"},
                    "content": {"type": "string", "description": "邮件内容（HTML）"},
                    "to": {"type": "array", "items": {"type": "object"}, "description": "收件人列表"},
                    "cc": {"type": "array", "items": {"type": "object"}, "description": "抄送人列表"},
                    "bcc": {"type": "array", "items": {"type": "object"}, "description": "密送人列表"},
                    "reply_to_mail_id": {"type": "string", "description": "回复邮件ID（回复时使用）"}
                },
                "required": ["mailbox_id", "subject", "content", "to"]
            }),
            method: Method::POST,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/drafts",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "subject": args["subject"],
                    "content": args["content"],
                    "to": args["to"],
                });
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_list_mail",
            description: "列出飞书邮箱邮件",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID"},
                    "folder_id": {"type": "string", "description": "文件夹ID", "default": "INBOX"},
                    "page_size": {"type": "integer", "description": "每页数量", "default": 20},
                    "page_token": {"type": "string", "description": "翻页token"}
                },
                "required": ["mailbox_id"]
            }),
            method: Method::GET,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/messages",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                let mut query = serde_json::Map::new();
                if let Some(v) = args.get("folder_id") { query.insert("folder_id".into(), v.clone()); }
                if let Some(v) = args.get("page_size") { query.insert("page_size".into(), v.clone()); }
                if let Some(v) = args.get("page_token") { query.insert("page_token".into(), v.clone()); }
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id)]),
                    query: if query.is_empty() { None } else { Some(json!(query)) },
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_get_mail",
            description: "读取飞书邮件详情",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID"},
                    "message_id": {"type": "string", "description": "邮件ID"}
                },
                "required": ["mailbox_id", "message_id"]
            }),
            method: Method::GET,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/messages/{message_id}",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                let message_id = args["message_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id), ("message_id", message_id)]),
                    query: None,
                    body: None,
                }
            },
        },
        ToolSpec {
            name: "feishu_reply_mail",
            description: "回复飞书邮件",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID"},
                    "message_id": {"type": "string", "description": "被回复邮件ID"},
                    "content": {"type": "string", "description": "回复内容（HTML）"},
                    "reply_all": {"type": "boolean", "description": "是否回复全部", "default": false}
                },
                "required": ["mailbox_id", "message_id", "content"]
            }),
            method: Method::POST,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/drafts",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "reply_to_mail_id": args["message_id"],
                    "content": args["content"],
                });
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_forward_mail",
            description: "转发飞书邮件",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID"},
                    "message_id": {"type": "string", "description": "被转发邮件ID"},
                    "to": {"type": "array", "items": {"type": "object"}, "description": "收件人"},
                    "content": {"type": "string", "description": "附言（可选）"}
                },
                "required": ["mailbox_id", "message_id", "to"]
            }),
            method: Method::POST,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/drafts",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                let body = json!({
                    "forward_to_mail_id": args["message_id"],
                    "to": args["to"],
                });
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id)]),
                    query: None,
                    body: Some(body),
                }
            },
        },
        ToolSpec {
            name: "feishu_triage_mail",
            description: "飞书邮件摘要/分类（列出未读邮件）",
            schema: json!({
                "type": "object",
                "properties": {
                    "mailbox_id": {"type": "string", "description": "邮箱ID"},
                    "page_size": {"type": "integer", "description": "返回数量", "default": 10}
                },
                "required": ["mailbox_id"]
            }),
            method: Method::GET,
            path: "open-apis/mail/v1/user_mailboxes/{mailbox_id}/messages",
            param_mapper: |args| {
                let mailbox_id = args["mailbox_id"].as_str().unwrap_or("").to_string();
                MappedParams {
                    path_params: HashMap::from([("mailbox_id", mailbox_id)]),
                    query: Some(json!({ "page_size": args["page_size"].as_i64().unwrap_or(10) })),
                    body: None,
                }
            },
        },
    ]
}
