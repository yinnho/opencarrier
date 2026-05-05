//! xiaohongshu-mcp — Xiaohongshu Creator MCP Server (multi-tenant)
//!
//! Each tool call carries `cookie`, allowing a single MCP server to serve
//! multiple Xiaohongshu creator accounts simultaneously.

use anyhow::Result;
use mcp_common::cookie::make_cookie;
use mcp_common::{define_params, impl_cookie, json::json_to_string};
use reqwest::Method;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, transport::stdio as stdio_transport, ServiceExt};

mod api;

// ================================================================== //
//  Parameter structs                                                   //
// ================================================================== //

define_params!(NotesParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(NotesParams);

define_params!(NoteDetailParams {
    #[schemars(description = "笔记ID")]
    note_id: String,
});
impl_cookie!(NoteDetailParams);

define_params!(ProfileParams {});
impl_cookie!(ProfileParams);

define_params!(StatsParams {
    #[schemars(description = "")]
    period: Option<String>,
});
impl_cookie!(StatsParams);

define_params!(NotesSummaryParams {
    #[schemars(description = "")]
    limit: Option<i64>,
});
impl_cookie!(NotesSummaryParams);

// ================================================================== //
//  MCP Server                                                          //
// ================================================================== //

#[derive(Clone)]
struct XiaohongshuServer;

#[tool_router(server_handler)]
impl XiaohongshuServer {
    #[tool(description = "获取小红书创作者笔记列表")]
    async fn xhs_creator_notes(
        &self,
        Parameters(params): Parameters<NotesParams>,
    ) -> String {
        let limit = params.limit.unwrap_or(20);
        let query = format!("type=0&page_size={limit}&page_num=1");
        let path = format!("/api/galaxy/creator/datacenter/note/analyze/list?{query}");
        match api::xhs_api(&make_cookie(&params), &path, Method::GET).await {
            Ok(resp) => {
                let notes = resp.pointer("/data/data")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter().map(|note| {
                            serde_json::json!({
                                "id": note.get("id"),
                                "title": note.get("title"),
                                "post_time": note.get("post_time"),
                                "read_count": note.get("read_count"),
                                "like_count": note.get("like_count"),
                                "fav_count": note.get("fav_count"),
                                "comment_count": note.get("comment_count"),
                            })
                        }).collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                json_to_string(&serde_json::Value::Array(notes))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取小红书单篇笔记详情")]
    async fn xhs_creator_note_detail(
        &self,
        Parameters(params): Parameters<NoteDetailParams>,
    ) -> String {
        let query = format!("note_id={}", params.note_id);
        let path = format!("/api/galaxy/creator/datacenter/note/base?{query}");
        match api::xhs_api(&make_cookie(&params), &path, Method::GET).await {
            Ok(resp) => {
                let data = resp.pointer("/data/data").cloned()
                    .unwrap_or_else(|| serde_json::json!({"error": "Note not found"}));
                json_to_string(&data)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取小红书创作者账号信息")]
    async fn xhs_creator_profile(
        &self,
        Parameters(params): Parameters<ProfileParams>,
    ) -> String {
        match api::xhs_api(&make_cookie(&params), "/api/galaxy/creator/home/personal_info", Method::GET).await {
            Ok(resp) => {
                let data = resp.pointer("/data/data");
                let result = match data {
                    Some(d) => serde_json::json!({
                        "name": d.get("name"),
                        "fans_count": d.get("fans_count"),
                        "follow_count": d.get("follow_count"),
                        "faved_count": d.get("faved_count"),
                        "personal_desc": d.get("personal_desc"),
                        "level": d.pointer("/grow_info/level"),
                    }),
                    None => serde_json::json!({"error": "Profile not found"}),
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取小红书数据总览")]
    async fn xhs_creator_stats(
        &self,
        Parameters(params): Parameters<StatsParams>,
    ) -> String {
        match api::xhs_api(&make_cookie(&params), "/api/galaxy/creator/data/note_detail_new", Method::GET).await {
            Ok(resp) => {
                let data = resp.pointer("/data/data");
                let result = match data {
                    Some(d) => {
                        let seven = d.get("seven").cloned().unwrap_or(serde_json::Value::Null);
                        let thirty = d.get("thirty").cloned().unwrap_or(serde_json::Value::Null);
                        serde_json::json!({
                            "seven": seven,
                            "thirty": thirty,
                        })
                    }
                    None => serde_json::json!({"error": "Stats not found"}),
                };
                json_to_string(&result)
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "获取小红书笔记批量摘要（笔记列表+详情汇总）")]
    async fn xhs_creator_notes_summary(
        &self,
        Parameters(params): Parameters<NotesSummaryParams>,
    ) -> String {
        let cookie = make_cookie(&params);
        let limit = params.limit.unwrap_or(3);

        // Step 1: fetch notes list
        let list_query = format!("type=0&page_size={limit}&page_num=1");
        let list_path = format!("/api/galaxy/creator/datacenter/note/analyze/list?{list_query}");
        let list_resp = match api::xhs_api(&cookie, &list_path, Method::GET).await {
            Ok(r) => r,
            Err(e) => return format!("{{\"error\": \"{}\"}}", e),
        };

        let notes = list_resp.pointer("/data/data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        if notes.is_empty() {
            return json_to_string(&serde_json::json!({"notes": [], "summary": "No notes found"}));
        }

        let mut summaries = Vec::new();
        for note in notes.iter().take(limit as usize) {
            let note_id = note.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let title = note.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let read_count = note.get("read_count").and_then(|v| v.as_i64()).unwrap_or(0);
            let like_count = note.get("like_count").and_then(|v| v.as_i64()).unwrap_or(0);
            let comment_count = note.get("comment_count").and_then(|v| v.as_i64()).unwrap_or(0);

            // Fetch detail
            let detail_path = format!("/api/galaxy/creator/datacenter/note/base?note_id={note_id}");
            let detail = match api::xhs_api(&cookie, &detail_path, Method::GET).await {
                Ok(r) => r.pointer("/data/data").cloned().unwrap_or(serde_json::Value::Null),
                Err(_) => serde_json::Value::Null,
            };

            summaries.push(serde_json::json!({
                "id": note_id,
                "title": title,
                "read_count": read_count,
                "like_count": like_count,
                "comment_count": comment_count,
                "detail": detail,
            }));
        }

        json_to_string(&serde_json::json!({"notes": summaries}))
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
            tracing_subscriber::EnvFilter::try_from_env("XIAOHONGSHU_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    tracing::info!("xiaohongshu-mcp starting (stdio, multi-tenant)");
    let server = XiaohongshuServer;
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;

    Ok(())
}
