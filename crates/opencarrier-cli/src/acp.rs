//! ACP (Agent Client Protocol) support for serve mode.
//!
//! Implements the standard ACP protocol over JSON-RPC 2.0, enabling opencarrier
//! to be used as an ACP-compatible Agent that can be connected to aginx and
//! other ACP clients (GitHub Copilot, Gemini CLI, etc.).
//!
//! See: https://agentclientprotocol.com

use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_memory::acp_session::AcpSessionStore;
use opencarrier_runtime::llm_driver::StreamEvent;
use opencarrier_types::agent::AgentId;
use opencarrier_types::message::StopReason;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::serve::{
    jsonrpc_error, jsonrpc_success, write_response, JsonRpcRequest, Response, SharedWriter,
    INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND,
};

// ---------------------------------------------------------------------------
// ACP Types
// ---------------------------------------------------------------------------

/// ACP content block in a prompt (session/prompt params.prompt[]).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum AcpContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "audio")]
    Audio { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource {
        resource: AcpResource,
    },
    #[serde(rename = "resource_link")]
    ResourceLink { uri: String, name: String },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AcpResource {
    #[allow(dead_code)]
    pub uri: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

// ---------------------------------------------------------------------------
// ACP Session State
// ---------------------------------------------------------------------------

/// An active ACP session, mapping sessionId to an opencarrier agent.
#[allow(dead_code)]
pub struct AcpSession {
    pub agent_id: AgentId,
    pub cwd: String,
    pub mcp_servers: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Tracks an in-flight `session/prompt` for cancellation.
pub struct ActivePrompt {
    /// Set by `session/cancel` to signal the prompt thread to stop.
    pub cancelled: Arc<AtomicBool>,
    /// Set by the prompt thread when it finishes (normally or by cancellation).
    pub completed: Arc<AtomicBool>,
}

/// Connection-level state for the ACP protocol.
#[derive(Default)]
pub struct AcpConnectionState {
    /// Sessions created in this connection: session_id -> AcpSession.
    pub sessions: HashMap<String, AcpSession>,
    /// Whether initialize handshake completed.
    pub initialized: bool,
    /// Active prompt threads: session_id -> cancel/completed flags.
    pub active_prompts: HashMap<String, ActivePrompt>,
}

// ---------------------------------------------------------------------------
// Method Routing
// ---------------------------------------------------------------------------

/// Returns true if the method name belongs to the ACP protocol.
pub fn is_acp_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "authenticate"
            | "session/new"
            | "session/load"
            | "session/prompt"
            | "session/cancel"
            | "session/list"
            | "session/set_mode"
            | "session/request_permission"
            // Aginx extensions (also called without _aginx/ prefix by some clients)
            | "listConversations"
            | "getMessages"
            | "deleteConversation"
            | "listAgents"
    ) || method.starts_with("_aginx/")
}

// ---------------------------------------------------------------------------
// Handler Entry Point
// ---------------------------------------------------------------------------

/// Handle an ACP protocol request.
///
/// Returns `None` when the handler writes its own response (session/prompt,
/// session/cancel). Returns `Some(Response)` for synchronous handlers whose
/// response the main loop should write.
pub fn handle_acp_request(
    kernel: &Arc<OpenCarrierKernel>,
    rt: &Arc<tokio::runtime::Runtime>,
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
    writer: &SharedWriter,
    store: &AcpSessionStore,
) -> Option<Response> {
    // Validate jsonrpc version
    if req.jsonrpc != "2.0" {
        return Some(jsonrpc_error(
            req.id.clone(),
            crate::serve::INVALID_REQUEST,
            "Invalid jsonrpc version",
        ));
    }

    // Except for initialize and authenticate, all ACP methods require initialization
    if req.method != "initialize" && req.method != "authenticate" && !state.initialized {
        return Some(jsonrpc_error(
            req.id.clone(),
            crate::serve::INVALID_REQUEST,
            "Not initialized: call initialize first",
        ));
    }

    match req.method.as_str() {
        "initialize" => acp_initialize(req, state),
        "authenticate" => {
            // opencarrier doesn't require authentication
            Some(jsonrpc_success(req.id.clone(), json!({})))
        }
        "session/new" => acp_session_new(kernel, req, state, store),
        "session/prompt" => acp_session_prompt(kernel, rt, req, state, writer, store),
        "session/load" => acp_session_load(kernel, req, state, writer, store),
        "session/cancel" => acp_session_cancel(req, state),
        "session/list" => acp_session_list(req, store),
        "session/set_mode" => acp_session_set_mode(req),
        "session/request_permission" => Some(jsonrpc_error(
            req.id.clone(),
            METHOD_NOT_FOUND,
            "session/request_permission not supported",
        )),
        "_aginx/listAgents" | "listAgents" => acp_aginx_list_agents(kernel, req),
        "_aginx/listConversations" | "listConversations" => {
            acp_aginx_list_conversations(req, store)
        }
        "_aginx/getMessages" | "getMessages" => acp_aginx_get_messages(req, store),
        "_aginx/deleteConversation" | "deleteConversation" => {
            acp_aginx_delete_conversation(req, state, store)
        }
        _ => Some(jsonrpc_error(
            req.id.clone(),
            METHOD_NOT_FOUND,
            &format!("ACP method not found: {}", req.method),
        )),
    }
}

// ---------------------------------------------------------------------------
// ACP Method Handlers
// ---------------------------------------------------------------------------

/// initialize — handshake, negotiate protocol version and exchange capabilities.
fn acp_initialize(req: &JsonRpcRequest, state: &mut AcpConnectionState) -> Option<Response> {
    let params = req.params.as_ref();

    let version = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1);

    let client_info = params.and_then(|p| p.get("clientInfo"));
    if let Some(info) = client_info {
        let name = info.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let ver = info.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        info!("[acp] initialize from {} v{}", name, ver);
    }

    state.initialized = true;

    Some(jsonrpc_success(
        req.id.clone(),
        json!({
            "protocolVersion": version,
            "agentInfo": {
                "name": "opencarrier",
                "title": "OpenCarrier",
                "version": env!("CARGO_PKG_VERSION")
            },
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {
                    "image": false,
                    "audio": false,
                    "embeddedContext": true
                },
                "mcpCapabilities": {
                    "http": false,
                    "sse": false
                },
                "sessionCapabilities": {}
            },
            "authMethods": []
        }),
    ))
}

/// session/new — create a new session, map to an opencarrier agent.
fn acp_session_new(
    kernel: &Arc<OpenCarrierKernel>,
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
    store: &AcpSessionStore,
) -> Option<Response> {
    let params = req.params.as_ref();

    let cwd = params
        .and_then(|p| p.get("cwd"))
        .and_then(|v| v.as_str())
        .unwrap_or("/");

    let mcp_servers = params
        .and_then(|p| p.get("mcpServers"))
        .cloned()
        .unwrap_or(serde_json::json!([]));

    // Resolve agent: requires _meta.aginx/agentId
    let agent_id = match resolve_agent_id(kernel, params) {
        Ok(id) => id,
        Err(msg) => {
            return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, &msg));
        }
    };

    let session_id = format!("sess_{}", uuid::Uuid::new_v4().simple());

    info!(
        "[acp] session/new: sessionId={}, agentId={}, cwd={}",
        session_id, agent_id, cwd
    );

    if let Err(e) = store.create_session(&session_id, &agent_id.to_string(), cwd) {
        error!("[acp] Failed to persist session: {e}");
        return Some(jsonrpc_error(
            req.id.clone(),
            INTERNAL_ERROR,
            &format!("Failed to persist session: {e}"),
        ));
    }

    state.sessions.insert(
        session_id.clone(),
        AcpSession {
            agent_id,
            cwd: cwd.to_string(),
            mcp_servers,
            created_at: chrono::Utc::now(),
        },
    );

    Some(jsonrpc_success(
        req.id.clone(),
        json!({
            "sessionId": session_id,
            "modes": {
                "currentModeId": "code",
                "availableModes": [
                    {"id": "code", "name": "Code", "description": "Write and modify code"},
                    {"id": "ask", "name": "Ask", "description": "Ask questions without changes"}
                ]
            }
        }),
    ))
}

/// session/prompt — send message with streaming session/update notifications.
///
/// Spawns a worker thread so the main loop stays responsive for session/cancel.
fn acp_session_prompt(
    kernel: &Arc<OpenCarrierKernel>,
    rt: &Arc<tokio::runtime::Runtime>,
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
    writer: &SharedWriter,
    store: &AcpSessionStore,
) -> Option<Response> {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, "Missing params")),
    };
    let session_id = match params.get("sessionId").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                "Missing sessionId",
            ))
        }
    };

    let session = match state.sessions.get(session_id) {
        Some(s) => s,
        None => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                &format!("Session not found: {}", session_id),
            ))
        }
    };

    // Reject if a prompt is already active on this session
    if let Some(active) = state.active_prompts.get(session_id) {
        if !active.completed.load(Ordering::Relaxed) {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                "Prompt already in progress for this session",
            ));
        }
    }

    // Extract text from prompt content blocks
    let prompt: Vec<AcpContentBlock> = params
        .get("prompt")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let message: String = prompt
        .iter()
        .filter_map(|b| match b {
            AcpContentBlock::Text { text } => Some(text.as_str()),
            AcpContentBlock::Resource { resource } => resource.text.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if message.is_empty() {
        return Some(jsonrpc_error(
            req.id.clone(),
            INVALID_PARAMS,
            "Empty prompt",
        ));
    }

    let agent_id = session.agent_id;
    let cwd = session.cwd.clone();

    // Persist user message before streaming
    if let Err(e) = store.append_user_message(session_id, &message, Some(&cwd)) {
        return Some(jsonrpc_error(
            req.id.clone(),
            INTERNAL_ERROR,
            &format!("Failed to append user message: {e}"),
        ));
    }

    // Create cancel/completed tokens
    let cancelled = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    state.active_prompts.insert(
        session_id.to_string(),
        ActivePrompt {
            cancelled: cancelled.clone(),
            completed: completed.clone(),
        },
    );

    // Clone everything for the worker thread
    let kernel = kernel.clone();
    let rt = rt.clone();
    let writer = writer.clone();
    let store = store.clone();
    let req_id = req.id.clone();
    let session_id_owned = session_id.to_string();

    std::thread::spawn(move || {
        let kernel_handle = kernel.get_kernel_handle();

        let result = rt.block_on(async {
            let (mut rx, handle) = match kernel
                .send_message_streaming(agent_id, &message, kernel_handle, None, None)
            {
                Ok(r) => r,
                Err(e) => return Err(format!("Failed to start streaming: {e}")),
            };

            // Track tool call IDs as FIFO: (tool_name, tool_call_id)
            let mut tool_id_fifo: Vec<(String, String)> = Vec::new();

            let mut stop_reason_str = "end_turn".to_string();
            let mut assistant_text = String::new();

            while let Some(event) = rx.recv().await {
                // Check cancellation
                if cancelled.load(Ordering::Relaxed) {
                    stop_reason_str = "cancelled".to_string();
                    break;
                }

                if let StreamEvent::TextDelta { text } = &event {
                    assistant_text.push_str(text);
                }

                if let Some(notification) =
                    map_stream_event_to_acp(&session_id_owned, &event, &mut tool_id_fifo)
                {
                    let mut w = writer.lock().unwrap();
                    if let Ok(json_line) = serde_json::to_string(&notification) {
                        let _ = writeln!(w, "{}", json_line);
                        let _ = w.flush();
                    }
                }

                if let StreamEvent::ContentComplete { stop_reason, .. } = &event {
                    stop_reason_str = match stop_reason {
                        StopReason::EndTurn | StopReason::StopSequence => "end_turn".to_string(),
                        StopReason::MaxTokens => "max_tokens".to_string(),
                        StopReason::ToolUse => "max_turn_requests".to_string(),
                    };
                }
            }

            // Wait for agent loop to finish post-processing
            let _ = handle.await;

            Ok((stop_reason_str, assistant_text))
        });

        // Persist assistant message
        if let Ok((_, ref text)) = result {
            if !text.is_empty() {
                let _ = store.append_assistant_message(&session_id_owned, text);
            }
        }

        // Write final response
        let final_response = match result {
            Ok((stop_reason, _)) => {
                jsonrpc_success(req_id.clone(), json!({"stopReason": stop_reason}))
            }
            Err(e) => {
                error!("[acp] session/prompt error: {e}");
                jsonrpc_error(req_id.clone(), INTERNAL_ERROR, &e)
            }
        };

        write_response(&writer, final_response);

        // Mark as completed
        completed.store(true, Ordering::Relaxed);
    });

    None // Worker thread writes the response
}

/// session/load — replay conversation history, then return result:null.
///
/// Per ACP spec, the agent must replay all messages via session/update
/// notifications (user_message_chunk / agent_message_chunk), then respond.
fn acp_session_load(
    kernel: &Arc<OpenCarrierKernel>,
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
    writer: &SharedWriter,
    store: &AcpSessionStore,
) -> Option<Response> {
    let params = req.params.as_ref();
    let session_id = match params
        .and_then(|p| p.get("sessionId"))
        .and_then(|v| v.as_str())
    {
        Some(id) => id,
        None => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                "Missing sessionId",
            ))
        }
    };

    let cwd = params
        .and_then(|p| p.get("cwd"))
        .and_then(|v| v.as_str())
        .unwrap_or("/");
    let mcp_servers = params
        .and_then(|p| p.get("mcpServers"))
        .cloned()
        .unwrap_or(json!([]));

    // Load session meta from persistence
    let meta = match store.get_session_meta(session_id) {
        Some(m) => m,
        None => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                &format!("Session not found: {}", session_id),
            ))
        }
    };

    // Resolve agent from stored meta
    let agent_id = match meta.agent_id.parse::<AgentId>() {
        Ok(id) => id,
        Err(_) => match kernel.registry.find_by_name(&meta.agent_id) {
            Some(entry) => entry.id,
            None => {
                return Some(jsonrpc_error(
                    req.id.clone(),
                    INVALID_PARAMS,
                    &format!("Agent not found: {}", meta.agent_id),
                ))
            }
        },
    };

    // Re-add session to connection state
    let created_at = chrono::DateTime::from_timestamp_millis(meta.created_at as i64)
        .unwrap_or(chrono::Utc::now());
    state.sessions.insert(
        session_id.to_string(),
        AcpSession {
            agent_id,
            cwd: cwd.to_string(),
            mcp_servers,
            created_at,
        },
    );

    info!(
        "[acp] session/load: sessionId={}, agentId={}, replaying history",
        session_id, agent_id
    );

    // Load messages and replay as session/update notifications
    let messages = match store.get_messages(session_id, 1000) {
        Ok(msgs) => msgs,
        Err(e) => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INTERNAL_ERROR,
                &format!("Failed to load messages: {e}"),
            ))
        }
    };

    {
        let mut w = writer.lock().unwrap();
        for msg in &messages {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let update_type = match role {
                "user" => "user_message_chunk",
                _ => "agent_message_chunk",
            };
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

            let notification = json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": update_type,
                        "content": {"type": "text", "text": content}
                    }
                }
            });

            if let Ok(json_line) = serde_json::to_string(&notification) {
                let _ = writeln!(w, "{}", json_line);
            }
        }
        let _ = w.flush();
    }

    Some(jsonrpc_success(req.id.clone(), Value::Null))
}

/// session/cancel — cancel an in-flight prompt (notification, no response).
///
/// Sets the cancelled flag so the prompt worker thread stops streaming
/// and returns `{"stopReason": "cancelled"}`.
fn acp_session_cancel(
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
) -> Option<Response> {
    let session_id = req
        .params
        .as_ref()
        .and_then(|p| p.get("sessionId"))
        .and_then(|v| v.as_str());

    if let Some(id) = session_id {
        if let Some(active) = state.active_prompts.get(id) {
            if !active.completed.load(Ordering::Relaxed) {
                active.cancelled.store(true, Ordering::Relaxed);
                debug!("[acp] Cancelled prompt for session {}", id);
            }
        }
    }

    None // No response — this is a notification
}

/// session/list — list all persisted sessions.
fn acp_session_list(
    req: &JsonRpcRequest,
    store: &AcpSessionStore,
) -> Option<Response> {
    match store.list_sessions() {
        Ok(sessions) => Some(jsonrpc_success(
            req.id.clone(),
            json!({ "sessions": sessions, "nextCursor": null }),
        )),
        Err(e) => Some(jsonrpc_error(
            req.id.clone(),
            INTERNAL_ERROR,
            &format!("Failed to list sessions: {e}"),
        )),
    }
}

/// session/set_mode — accept mode switch (opencarrier doesn't distinguish internally).
fn acp_session_set_mode(req: &JsonRpcRequest) -> Option<Response> {
    let params = req.params.as_ref();
    let mode = params
        .and_then(|p| p.get("modeId"))
        .and_then(|v| v.as_str())
        .unwrap_or("code");

    debug!("[acp] session/set_mode: {}", mode);

    Some(jsonrpc_success(req.id.clone(), json!({})))
}

/// _aginx/listAgents — list opencarrier agents.
fn acp_aginx_list_agents(
    kernel: &Arc<OpenCarrierKernel>,
    req: &JsonRpcRequest,
) -> Option<Response> {
    let agents: Vec<Value> = kernel
        .registry
        .list()
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id.to_string(),
                "name": entry.name,
                "version": entry.manifest.version,
                "description": entry.manifest.description
            })
        })
        .collect();

    Some(jsonrpc_success(
        req.id.clone(),
        json!({ "agents": agents }),
    ))
}

/// _aginx/listConversations — list conversations from persisted sessions.
fn acp_aginx_list_conversations(
    req: &JsonRpcRequest,
    store: &AcpSessionStore,
) -> Option<Response> {
    match store.list_sessions() {
        Ok(sessions) => {
            let conversations: Vec<Value> = sessions
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.get("sessionId").cloned().unwrap_or_default(),
                        "title": s.get("title").cloned().unwrap_or_else(|| json!("Untitled")),
                        "createdAt": s.get("createdAt").cloned().unwrap_or_default(),
                        "updatedAt": s.get("updatedAt").cloned().unwrap_or_default()
                    })
                })
                .collect();
            Some(jsonrpc_success(
                req.id.clone(),
                json!({ "conversations": conversations }),
            ))
        }
        Err(e) => Some(jsonrpc_error(
            req.id.clone(),
            INTERNAL_ERROR,
            &format!("Failed to list conversations: {e}"),
        )),
    }
}

/// _aginx/getMessages — get messages for a conversation (session).
fn acp_aginx_get_messages(
    req: &JsonRpcRequest,
    store: &AcpSessionStore,
) -> Option<Response> {
    let params = req.params.as_ref();
    let conversation_id = params
        .and_then(|p| p.get("conversationId"))
        .and_then(|v| v.as_str());

    let session_id = match conversation_id {
        Some(id) => id,
        None => {
            return Some(jsonrpc_error(
                req.id.clone(),
                INVALID_PARAMS,
                "Missing conversationId",
            ))
        }
    };

    let limit = params
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    match store.get_messages(session_id, limit) {
        Ok(messages) => Some(jsonrpc_success(
            req.id.clone(),
            json!({ "messages": messages }),
        )),
        Err(e) => {
            error!("[acp] getMessages error for session {}: {e}", session_id);
            Some(jsonrpc_success(
                req.id.clone(),
                json!({ "messages": [] }),
            ))
        }
    }
}

/// _aginx/deleteConversation — delete a conversation (remove from state and store).
fn acp_aginx_delete_conversation(
    req: &JsonRpcRequest,
    state: &mut AcpConnectionState,
    store: &AcpSessionStore,
) -> Option<Response> {
    let params = req.params.as_ref();
    let conversation_id = params
        .and_then(|p| p.get("conversationId"))
        .and_then(|v| v.as_str());

    if let Some(id) = conversation_id {
        let _ = store.delete_session(id);
        state.sessions.remove(id);
    }

    Some(jsonrpc_success(req.id.clone(), json!({})))
}

// ---------------------------------------------------------------------------
// StreamEvent → ACP Notification Mapping
// ---------------------------------------------------------------------------

/// Map an internal StreamEvent to an ACP session/update notification.
/// Returns None for events that should not be forwarded.
fn map_stream_event_to_acp(
    session_id: &str,
    event: &StreamEvent,
    tool_id_fifo: &mut Vec<(String, String)>,
) -> Option<Value> {
    let update = match event {
        StreamEvent::TextDelta { text } => Some(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": text}
        })),

        StreamEvent::ThinkingDelta { text } => Some(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": text}
        })),

        StreamEvent::ToolUseStart { id, name } => {
            // Track this tool call for later ToolExecutionResult matching
            tool_id_fifo.push((name.clone(), id.clone()));
            Some(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": format_tool_title(name),
                "kind": map_tool_kind(name),
                "status": "pending"
            }))
        }

        StreamEvent::ToolUseEnd { id, name, input } => {
            let title = format_tool_title(name);
            let raw_input = if input.is_null() || input == &json!({}) {
                None
            } else {
                Some(input.clone())
            };
            Some(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": "in_progress",
                "title": title,
                "rawInput": raw_input
            }))
        }

        StreamEvent::ToolExecutionResult {
            id: ref tool_evt_id,
            name,
            result_preview,
            is_error,
        } => {
            // Prefer exact ID match, fall back to FIFO by name
            let call_id = tool_id_fifo
                .iter()
                .position(|(_, tid)| tid == tool_evt_id)
                .map(|pos| tool_id_fifo.remove(pos).1)
                .unwrap_or_else(|| {
                    tool_id_fifo
                        .iter()
                        .position(|(n, _)| n == name)
                        .map(|pos| tool_id_fifo.remove(pos).1)
                        .unwrap_or_else(|| tool_evt_id.clone())
                });
            let status = if *is_error { "failed" } else { "completed" };
            Some(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call_id,
                "status": status,
                "content": [{
                    "type": "content",
                    "content": {"type": "text", "text": result_preview}
                }]
            }))
        }

        // Skip internal events
        StreamEvent::ContentComplete { .. } => None,
        StreamEvent::PhaseChange { .. } => None,
        StreamEvent::ToolInputDelta { .. } => None,
    };

    update.map(|u| {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": u
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve agent ID from session/new params.
/// Requires _meta.aginx/agentId; returns an error if missing or unknown.
fn resolve_agent_id(
    kernel: &OpenCarrierKernel,
    params: Option<&Value>,
) -> Result<AgentId, String> {
    let explicit = params
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get("aginx/agentId"))
        .and_then(|v| v.as_str());

    if let Some(name) = explicit {
        if let Ok(id) = name.parse::<AgentId>() {
            return Ok(id);
        }
        if let Some(entry) = kernel.registry.find_by_name(name) {
            return Ok(entry.id);
        }
        return Err(format!("Agent not found: {}", name));
    }

    Err("Missing _meta.aginx/agentId in session/new params".to_string())
}

/// Map tool name to ACP ToolKind.
fn map_tool_kind(tool_name: &str) -> &'static str {
    match tool_name {
        n if n.contains("read")
            || n.contains("cat")
            || n.contains("head")
            || n.contains("file_read") =>
        {
            "read"
        }
        n if n.contains("write")
            || n.contains("edit")
            || n.contains("patch")
            || n.contains("file_write") =>
        {
            "edit"
        }
        n if n.contains("delete") || n.contains("remove") || n.contains("rm") => "delete",
        n if n.contains("move") || n.contains("rename") => "move",
        n if n.contains("search")
            || n.contains("grep")
            || n.contains("find")
            || n.contains("glob") =>
        {
            "search"
        }
        n if n.contains("bash")
            || n.contains("exec")
            || n.contains("run")
            || n.contains("shell") =>
        {
            "execute"
        }
        n if n.contains("fetch")
            || n.contains("curl")
            || n.contains("wget")
            || n.contains("http") =>
        {
            "fetch"
        }
        _ => "other",
    }
}

/// Format a human-readable tool title for ACP notifications.
fn format_tool_title(tool_name: &str) -> String {
    // Capitalize first letter, replace underscores with spaces
    let mut result = String::with_capacity(tool_name.len());
    for (i, c) in tool_name.chars().enumerate() {
        if i == 0 {
            for ch in c.to_uppercase() {
                result.push(ch);
            }
        } else if c == '_' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_acp_method() {
        assert!(is_acp_method("initialize"));
        assert!(is_acp_method("session/new"));
        assert!(is_acp_method("session/prompt"));
        assert!(is_acp_method("session/cancel"));
        assert!(is_acp_method("session/load"));
        assert!(is_acp_method("session/list"));
        assert!(is_acp_method("session/set_mode"));
        assert!(is_acp_method("_aginx/listAgents"));
        assert!(is_acp_method("_aginx/discoverRemote"));

        // Old methods should NOT match
        assert!(!is_acp_method("hello"));
        assert!(!is_acp_method("sendMessage"));
        assert!(!is_acp_method("getAgentCard"));
        assert!(!is_acp_method("bye"));
    }

    #[test]
    fn test_map_tool_kind() {
        assert_eq!(map_tool_kind("read_file"), "read");
        assert_eq!(map_tool_kind("file_write"), "edit");
        assert_eq!(map_tool_kind("bash"), "execute");
        assert_eq!(map_tool_kind("web_fetch"), "fetch");
        assert_eq!(map_tool_kind("search_files"), "search");
        assert_eq!(map_tool_kind("delete_file"), "delete");
        assert_eq!(map_tool_kind("unknown_tool"), "other");
    }

    #[test]
    fn test_format_tool_title() {
        assert_eq!(format_tool_title("read_file"), "Read file");
        assert_eq!(format_tool_title("bash"), "Bash");
        assert_eq!(format_tool_title("web_fetch"), "Web fetch");
    }

    #[test]
    fn test_acp_content_block_text() {
        let json = r#"{"type":"text","text":"Hello"}"#;
        let block: AcpContentBlock = serde_json::from_str(json).unwrap();
        match block {
            AcpContentBlock::Text { text } => assert_eq!(text, "Hello"),
            _ => panic!("Expected Text variant"),
        }
    }

    #[test]
    fn test_acp_content_block_resource() {
        let json = r#"{"type":"resource","resource":{"uri":"file:///test.rs","text":"fn main() {}"}}"#;
        let block: AcpContentBlock = serde_json::from_str(json).unwrap();
        match block {
            AcpContentBlock::Resource { resource } => {
                assert_eq!(resource.uri, "file:///test.rs");
                assert_eq!(resource.text.as_deref(), Some("fn main() {}"));
            }
            _ => panic!("Expected Resource variant"),
        }
    }

    #[test]
    fn test_map_stream_event_text_delta() {
        let mut tool_fifo = Vec::new();
        let event = StreamEvent::TextDelta {
            text: "Hello".to_string(),
        };
        let notification = map_stream_event_to_acp("sess_test", &event, &mut tool_fifo).unwrap();

        assert_eq!(notification["method"], "session/update");
        assert_eq!(notification["params"]["sessionId"], "sess_test");
        assert_eq!(
            notification["params"]["update"]["sessionUpdate"],
            "agent_message_chunk"
        );
        assert_eq!(notification["params"]["update"]["content"]["text"], "Hello");
    }

    #[test]
    fn test_map_stream_event_tool_call() {
        let mut tool_fifo = Vec::new();
        let event = StreamEvent::ToolUseStart {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
        };
        let notification = map_stream_event_to_acp("sess_test", &event, &mut tool_fifo).unwrap();

        assert_eq!(
            notification["params"]["update"]["sessionUpdate"],
            "tool_call"
        );
        assert_eq!(notification["params"]["update"]["toolCallId"], "call_1");
        assert_eq!(notification["params"]["update"]["kind"], "read");
        assert_eq!(notification["params"]["update"]["status"], "pending");
    }

    #[test]
    fn test_map_stream_event_thinking() {
        let mut tool_fifo = Vec::new();
        let event = StreamEvent::ThinkingDelta {
            text: "Let me think...".to_string(),
        };
        let notification = map_stream_event_to_acp("sess_test", &event, &mut tool_fifo).unwrap();

        assert_eq!(
            notification["params"]["update"]["sessionUpdate"],
            "agent_thought_chunk"
        );
        assert_eq!(
            notification["params"]["update"]["content"]["text"],
            "Let me think..."
        );
    }

    #[test]
    fn test_map_stream_event_skipped() {
        let mut tool_fifo = Vec::new();
        let event = StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        };
        assert!(map_stream_event_to_acp("sess_test", &event, &mut tool_fifo).is_none());

        let event = StreamEvent::PhaseChange {
            phase: "thinking".to_string(),
            detail: None,
        };
        assert!(map_stream_event_to_acp("sess_test", &event, &mut tool_fifo).is_none());
    }

    #[test]
    fn test_tool_id_fifo_parallel_calls() {
        let mut tool_fifo = Vec::new();

        // Two parallel read_file calls
        let e1 = StreamEvent::ToolUseStart {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
        };
        let _ = map_stream_event_to_acp("s", &e1, &mut tool_fifo);
        assert_eq!(tool_fifo.len(), 1);

        let e2 = StreamEvent::ToolUseStart {
            id: "call_2".to_string(),
            name: "read_file".to_string(),
        };
        let _ = map_stream_event_to_acp("s", &e2, &mut tool_fifo);
        assert_eq!(tool_fifo.len(), 2);

        // First result matches call_1
        let r1 = StreamEvent::ToolExecutionResult {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            result_preview: "content1".to_string(),
            is_error: false,
        };
        let n1 = map_stream_event_to_acp("s", &r1, &mut tool_fifo).unwrap();
        assert_eq!(n1["params"]["update"]["toolCallId"], "call_1");
        assert_eq!(tool_fifo.len(), 1); // consumed first entry

        // Second result matches call_2
        let r2 = StreamEvent::ToolExecutionResult {
            id: "call_2".to_string(),
            name: "read_file".to_string(),
            result_preview: "content2".to_string(),
            is_error: false,
        };
        let n2 = map_stream_event_to_acp("s", &r2, &mut tool_fifo).unwrap();
        assert_eq!(n2["params"]["update"]["toolCallId"], "call_2");
        assert_eq!(tool_fifo.len(), 0); // all consumed
    }
}
