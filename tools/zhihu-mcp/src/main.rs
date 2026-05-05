//! zhihu-mcp — Zhihu MCP Server (multi-tenant)
//!
//! Each tool call carries `cookie`, allowing a single MCP server to serve
//! multiple Zhihu accounts simultaneously.

use anyhow::Result;
use mcp_common::cookie::{make_cookie, CookieHolder};
use mcp_common::{define_params, impl_cookie, json::json_to_string};
use reqwest::Method;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, transport::stdio as stdio_transport, ServiceExt};
use serde_json::Value;

mod api;

// ================================================================== //
//  Parameter structs                                                   //
// ================================================================== //

define_params!(HotParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(HotParams);

define_params!(QuestionParams {
    #[schemars(description = "问题ID")]
    question_id: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(QuestionParams);

define_params!(SearchParams {
    #[schemars(description = "搜索关键词")]
    query: String,
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(SearchParams);

// ================================================================== //
//  Helpers                                                             //
// ================================================================== //

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
    result = result.replace("&nbsp;", " ");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&amp;", "&");
    result = result.replace("&#39;", "'");
    result = result.replace("&quot;", "\"");
    result
}

// ================================================================== //
//  MCP Server                                                          //
// ================================================================== //

#[derive(Clone)]
struct ZhihuServer;

#[tool_router(server_handler)]
impl ZhihuServer {
    #[tool(description = "获取知乎热榜")]
    async fn zhihu_hot(&self, Parameters(params): Parameters<HotParams>) -> String {
        let limit = params.limit.unwrap_or(20);
        let query = format!("limit={limit}");
        match api::zhihu_api(&make_cookie(&params), Method::GET, "/api/v3/feed/topstory/hot-lists/total", Some(&query)).await {
            Ok(resp) => {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Value = items.iter().take(limit as usize).enumerate().map(|(i, item)| {
                    let target = item.get("target");
                    serde_json::json!({
                        "rank": i + 1,
                        "title": target.and_then(|t| t.get("title")).and_then(|v| v.as_str()).unwrap_or(""),
                        "heat": item.get("detail_text").and_then(|v| v.as_str()).unwrap_or(""),
                        "answers": target.and_then(|t| t.get("answer_count")).and_then(|v| v.as_i64()).unwrap_or(0),
                        "url": target.and_then(|t| t.get("id"))
                            .map(|id| format!("https://www.zhihu.com/question/{}", id)),
                    })
                }).collect::<Vec<_>>().into();
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取知乎问题回答")]
    async fn zhihu_question(&self, Parameters(params): Parameters<QuestionParams>) -> String {
        let limit = params.limit.unwrap_or(5);
        let query = format!("limit={limit}&offset=0&sort_by=default&include=data[*].content,voteup_count,comment_count,author");
        let path = format!("/api/v4/questions/{}/answers", params.question_id);
        match api::zhihu_api(&make_cookie(&params), Method::GET, &path, Some(&query)).await {
            Ok(resp) => {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Value = items.iter().enumerate().map(|(i, item)| {
                    let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let plain = strip_html(content);
                    let truncated = if plain.len() > 200 { format!("{}...", &plain[..200]) } else { plain.clone() };
                    serde_json::json!({
                        "rank": i + 1,
                        "author": item.get("author").and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                        "votes": item.get("voteup_count").and_then(|v| v.as_i64()).unwrap_or(0),
                        "content": truncated,
                    })
                }).collect::<Vec<_>>().into();
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "搜索知乎内容")]
    async fn zhihu_search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let limit = params.limit.unwrap_or(10);
        let encoded: String = params.query.bytes().map(|b| {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{:02X}", b),
            }
        }).collect();
        let query = format!("q={encoded}&t=general&offset=0&limit={limit}");
        match api::zhihu_api(&make_cookie(&params), Method::GET, "/api/v4/search_v3", Some(&query)).await {
            Ok(resp) => {
                let items = resp.pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                let result: Value = items.iter()
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

                        serde_json::json!({
                            "rank": i + 1,
                            "title": title,
                            "type": obj_type,
                            "author": author,
                            "votes": votes,
                            "excerpt": if excerpt.len() > 100 { format!("{}...", &excerpt[..100]) } else { excerpt.to_string() },
                            "url": url,
                        })
                    }).collect::<Vec<_>>().into();
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }
}

// ================================================================== //
//  Entry point                                                         //
// ================================================================== //

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZHIHU_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    tracing::info!("zhihu-mcp starting (stdio, multi-tenant)");
    let server = ZhihuServer;
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;

    Ok(())
}
