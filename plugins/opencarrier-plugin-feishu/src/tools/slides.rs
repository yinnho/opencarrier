//! Slides (幻灯片) tools — 2 tools.

use super::{body_only, MappedParams, ToolSpec};
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;

pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "feishu_create_slides",
            description: "创建飞书幻灯片演示文稿",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "演示文稿标题"},
                    "folder_token": {"type": "string", "description": "所在文件夹token"}
                },
                "required": ["title"]
            }),
            method: Method::POST,
            path: "open-apis/slides/v1/xml_presentations",
            param_mapper: body_only,
        },
        ToolSpec {
            name: "feishu_replace_slide",
            description: "替换飞书幻灯片内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "演示文稿ID"},
                    "slide_id": {"type": "string", "description": "幻灯片ID"},
                    "replacements": {"type": "object", "description": "替换内容映射"}
                },
                "required": ["presentation_id", "slide_id", "replacements"]
            }),
            method: Method::POST,
            path: "open-apis/slides/v1/xml_presentations/{presentation_id}/slides/{slide_id}/replace",
            param_mapper: |args| {
                let presentation_id = args["presentation_id"].as_str().unwrap_or("").to_string();
                let slide_id = args["slide_id"].as_str().unwrap_or("").to_string();
                let body = json!({ "replacements": args["replacements"] });
                MappedParams {
                    path_params: HashMap::from([
                        ("presentation_id", presentation_id),
                        ("slide_id", slide_id),
                    ]),
                    query: None,
                    body: Some(body),
                }
            },
        },
    ]
}
