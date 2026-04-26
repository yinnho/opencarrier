//! Zhihu tool specifications — 3 tools backed by REST API.

use reqwest::Method;
use serde_json::{json, Value};

pub struct ZhihuToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub method: Method,
    pub path: &'static str,
    pub build_query: fn(&Value) -> Option<String>,
    pub parse_response: fn(&Value) -> Value,
}

fn strip_html(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Decode common entities
    result = result.replace("&nbsp;", " ");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&amp;", "&");
    result = result.replace("&#39;", "'");
    result = result.replace("&quot;", "\"");
    result
}

pub fn all_tools() -> Vec<ZhihuToolSpec> {
    vec![
        ZhihuToolSpec {
            name: "zhihu_hot",
            description: "获取知乎热榜",
            schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回数量", "default": 20}
                }
            }),
            method: Method::GET,
            path: "/api/v3/feed/topstory/hot-lists/total",
            build_query: |args| {
                Some(format!("limit={}", args["limit"].as_i64().unwrap_or(20)))
            },
            parse_response: |resp| {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                let limit = 20; // already limited by API
                items.iter().take(limit).enumerate().map(|(i, item)| {
                    let target = item.get("target");
                    json!({
                        "rank": i + 1,
                        "title": target.and_then(|t| t.get("title")).and_then(|v| v.as_str()).unwrap_or(""),
                        "heat": item.get("detail_text").and_then(|v| v.as_str()).unwrap_or(""),
                        "answers": target.and_then(|t| t.get("answer_count")).and_then(|v| v.as_i64()).unwrap_or(0),
                        "url": target.and_then(|t| t.get("id"))
                            .map(|id| format!("https://www.zhihu.com/question/{}", id)),
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        ZhihuToolSpec {
            name: "zhihu_question",
            description: "获取知乎问题回答",
            schema: json!({
                "type": "object",
                "properties": {
                    "question_id": {"type": "string", "description": "问题ID"},
                    "limit": {"type": "integer", "description": "返回回答数量", "default": 5}
                },
                "required": ["question_id"]
            }),
            method: Method::GET,
            path: "/api/v4/questions/{question_id}/answers",
            build_query: |args| {
                let limit = args["limit"].as_i64().unwrap_or(5);
                Some(format!("limit={limit}&offset=0&sort_by=default&include=data[*].content,voteup_count,comment_count,author"))
            },
            parse_response: |resp| {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter().enumerate().map(|(i, item)| {
                    let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let plain = strip_html(content);
                    let truncated = if plain.len() > 200 { format!("{}...", &plain[..200]) } else { plain.clone() };
                    json!({
                        "rank": i + 1,
                        "author": item.get("author").and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                        "votes": item.get("voteup_count").and_then(|v| v.as_i64()).unwrap_or(0),
                        "content": truncated,
                    })
                }).collect::<Vec<_>>().into()
            },
        },

        ZhihuToolSpec {
            name: "zhihu_search",
            description: "搜索知乎内容",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "limit": {"type": "integer", "description": "返回数量", "default": 10}
                },
                "required": ["query"]
            }),
            method: Method::GET,
            path: "/api/v4/search_v3",
            build_query: |args| {
                let q = args["query"].as_str().unwrap_or("");
                let encoded: String = q.bytes().map(|b| {
                    match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                        _ => format!("%{:02X}", b),
                    }
                }).collect();
                let limit = args["limit"].as_i64().unwrap_or(10);
                Some(format!("q={}&t=general&offset=0&limit={limit}", encoded))
            },
            parse_response: |resp| {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                items.iter()
                    .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("search_result"))
                    .enumerate().map(|(i, item)| {
                    let obj = item.get("object");
                    let obj_type = obj.and_then(|o| o.get("type")).and_then(|v| v.as_str()).unwrap_or("");
                    let title = obj.and_then(|o| {
                        o.get("title").or_else(|| o.get("question").and_then(|q| q.get("name")))
                    }).and_then(|v| v.as_str()).unwrap_or("");
                    let excerpt = obj.and_then(|o| o.get("excerpt")).and_then(|v| v.as_str()).unwrap_or("");
                    let author = obj.and_then(|o| o.get("author")).and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                    let votes = obj.and_then(|o| o.get("voteup_count")).and_then(|v| v.as_i64()).unwrap_or(0);

                    let url = match obj_type {
                        "answer" => {
                            let qid = obj.and_then(|o| o.get("question")).and_then(|q| q.get("id")).and_then(|v| v.as_str()).unwrap_or("");
                            let aid = obj.and_then(|o| o.get("id")).and_then(|v| v.as_str()).unwrap_or("");
                            format!("https://www.zhihu.com/question/{qid}/answer/{aid}")
                        }
                        "article" => {
                            let aid = obj.and_then(|o| o.get("id")).and_then(|v| v.as_str()).unwrap_or("");
                            format!("https://zhuanlan.zhihu.com/p/{aid}")
                        }
                        _ => {
                            let id = obj.and_then(|o| o.get("id")).and_then(|v| v.as_str()).unwrap_or("");
                            format!("https://www.zhihu.com/question/{id}")
                        }
                    };

                    json!({
                        "rank": i + 1,
                        "title": title,
                        "type": obj_type,
                        "author": author,
                        "votes": votes,
                        "excerpt": if excerpt.len() > 100 { format!("{}...", &excerpt[..100]) } else { excerpt.to_string() },
                        "url": url,
                    })
                }).collect::<Vec<_>>().into()
            },
        },
    ]
}
