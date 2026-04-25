//! Session management endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
/// GET /api/agents/:id/session — Get agent session (conversation history).
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match state.kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => {
            // Two-pass approach: ToolUse blocks live in Assistant messages while
            // ToolResult blocks arrive in subsequent User messages.  Pass 1
            // collects all tool_use entries keyed by id; pass 2 attaches results.

            // Pass 1: build messages and a lookup from tool_use_id → (msg_idx, tool_idx)
            use base64::Engine as _;
            let mut built_messages: Vec<serde_json::Value> = Vec::new();
            let mut tool_use_index: std::collections::HashMap<String, (usize, usize)> =
                std::collections::HashMap::new();

            for m in &session.messages {
                let mut tools: Vec<serde_json::Value> = Vec::new();
                let mut msg_images: Vec<serde_json::Value> = Vec::new();
                let content = match &m.content {
                    opencarrier_types::message::MessageContent::Text(t) => t.clone(),
                    opencarrier_types::message::MessageContent::Blocks(blocks) => {
                        let mut texts = Vec::new();
                        for b in blocks {
                            match b {
                                opencarrier_types::message::ContentBlock::Text { text, .. } => {
                                    texts.push(text.clone());
                                }
                                opencarrier_types::message::ContentBlock::Image {
                                    media_type,
                                    data,
                                } => {
                                    texts.push("[Image]".to_string());
                                    // Persist image to upload dir so it can be
                                    // served back when loading session history.
                                    let file_id = uuid::Uuid::new_v4().to_string();
                                    let upload_dir =
                                        std::env::temp_dir().join("opencarrier_uploads");
                                    let _ = std::fs::create_dir_all(&upload_dir);
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(data)
                                    {
                                        let _ = std::fs::write(upload_dir.join(&file_id), &bytes);
                                        UPLOAD_REGISTRY.insert(
                                            file_id.clone(),
                                            UploadMeta {
                                                content_type: media_type.clone(),
                                                tenant_id: ctx.tenant_id.clone(),
                                            },
                                        );
                                        msg_images.push(serde_json::json!({
                                            "file_id": file_id,
                                            "filename": format!("image.{}", media_type.rsplit('/').next().unwrap_or("png")),
                                        }));
                                    }
                                }
                                opencarrier_types::message::ContentBlock::ToolUse {
                                    id,
                                    name,
                                    input,
                                    ..
                                } => {
                                    let tool_idx = tools.len();
                                    tools.push(serde_json::json!({
                                        "name": name,
                                        "input": input,
                                        "running": false,
                                        "expanded": false,
                                    }));
                                    // Will be filled after this loop when we know msg_idx
                                    tool_use_index.insert(id.clone(), (usize::MAX, tool_idx));
                                }
                                // ToolResult blocks are handled in pass 2
                                opencarrier_types::message::ContentBlock::ToolResult { .. } => {}
                                _ => {}
                            }
                        }
                        texts.join("\n")
                    }
                };
                // Skip messages that are purely tool results (User role with only ToolResult blocks)
                if content.is_empty() && tools.is_empty() {
                    continue;
                }
                let msg_idx = built_messages.len();
                // Fix up the msg_idx for tool_use entries registered with sentinel
                for (_, (mi, _)) in tool_use_index.iter_mut() {
                    if *mi == usize::MAX {
                        *mi = msg_idx;
                    }
                }
                let mut msg = serde_json::json!({
                    "role": format!("{:?}", m.role),
                    "content": content,
                });
                if !tools.is_empty() {
                    msg["tools"] = serde_json::Value::Array(tools);
                }
                if !msg_images.is_empty() {
                    msg["images"] = serde_json::Value::Array(msg_images);
                }
                built_messages.push(msg);
            }

            // Pass 2: walk messages again and attach ToolResult to the correct tool
            for m in &session.messages {
                if let opencarrier_types::message::MessageContent::Blocks(blocks) = &m.content {
                    for b in blocks {
                        if let opencarrier_types::message::ContentBlock::ToolResult {
                            tool_use_id,
                            content: result,
                            is_error,
                            ..
                        } = b
                        {
                            if let Some(&(msg_idx, tool_idx)) = tool_use_index.get(tool_use_id) {
                                if let Some(msg) = built_messages.get_mut(msg_idx) {
                                    if let Some(tools_arr) =
                                        msg.get_mut("tools").and_then(|v| v.as_array_mut())
                                    {
                                        if let Some(tool_obj) = tools_arr.get_mut(tool_idx) {
                                            let preview: String =
                                                result.chars().take(2000).collect();
                                            tool_obj["result"] = serde_json::Value::String(preview);
                                            tool_obj["is_error"] =
                                                serde_json::Value::Bool(*is_error);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let messages = built_messages;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0,
                "context_window_tokens": 0,
                "messages": [],
            })),
        ),
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Session load failed"})),
            )
        }
    }
}
// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// GET /api/sessions — List all sessions with metadata.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() { None } else { ctx.tenant_id.as_deref() };
    match state.kernel.memory.list_sessions(tid) {
        Ok(sessions) => Json(serde_json::json!({"sessions": sessions})),
        Err(_) => Json(serde_json::json!({"sessions": []})),
    }
}
/// DELETE /api/sessions/:id — Delete a session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => opencarrier_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

    // Tenant check: resolve session → agent → agent's tenant
    match state.kernel.memory.get_session(session_id) {
        Ok(Some(session)) => {
            if let Some(entry) = state.kernel.registry.get(session.agent_id) {
                if !can_access(&ctx, entry.tenant_id.as_deref()) {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"error": "Access denied"})),
                    );
                }
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Session not found"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    }

    match state.kernel.memory.delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}
/// PUT /api/sessions/:id/label — Set a session label.
pub async fn set_session_label(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => opencarrier_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

    // Tenant check: resolve session → agent → agent's tenant
    match state.kernel.memory.get_session(session_id) {
        Ok(Some(session)) => {
            if let Some(entry) = state.kernel.registry.get(session.agent_id) {
                if !can_access(&ctx, entry.tenant_id.as_deref()) {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"error": "Access denied"})),
                    );
                }
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Session not found"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    }

    let label = req.get("label").and_then(|v| v.as_str());

    // Validate label if present
    if let Some(lbl) = label {
        if let Err(e) = opencarrier_types::agent::SessionLabel::new(lbl) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    }

    match state.kernel.memory.set_session_label(session_id, label) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}
/// GET /api/sessions/by-label/:label — Find session by label (scoped to agent).
pub async fn find_session_by_label(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((agent_id_str, label)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match resolve_agent_id_with_tenant(&agent_id_str, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match state.kernel.memory.find_session_by_label(agent_id, &label) {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No session found with that label"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}
// ── Multi-Session Endpoints ─────────────────────────────────────────────

/// GET /api/agents/{id}/sessions — List all sessions for an agent.
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
/// POST /api/agents/{id}/sessions — Create a new session for an agent.
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
/// POST /api/agents/{id}/sessions/{session_id}/switch — Switch to an existing session.
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, session_id_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => opencarrier_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            )
        }
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
// ── Extended Chat Command API Endpoints ─────────────────────────────────

/// POST /api/agents/{id}/session/reset — Reset an agent's session.
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
/// DELETE /api/agents/{id}/history — Clear ALL conversation history for an agent.
pub async fn clear_agent_history(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.kernel.clear_agent_history(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
/// POST /api/agents/{id}/session/compact — Trigger LLM session compaction.
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/agents/{id}/history", routing::delete(clear_agent_history))
        .route("/api/agents/{id}/session", routing::get(get_agent_session))
        .route("/api/agents/{id}/session/compact", routing::post(compact_session))
        .route("/api/agents/{id}/session/reset", routing::post(reset_session))
        .route("/api/agents/{id}/sessions", routing::post(create_agent_session).get(list_agent_sessions))
        .route("/api/agents/{id}/sessions/by-label/{label}", routing::get(find_session_by_label))
        .route("/api/agents/{id}/sessions/{session_id}/switch", routing::post(switch_agent_session))
        .route("/api/sessions", routing::get(list_sessions))
        .route("/api/sessions/by-label/{label}", routing::get(find_session_by_label))
        .route("/api/sessions/{id}", routing::delete(delete_session))
        .route("/api/sessions/{id}/label", routing::put(set_session_label))
}
