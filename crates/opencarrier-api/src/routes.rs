//! Route handlers for the OpenCarrier API.

use crate::types::*;
use axum::extract::{Path, Query, State};
use fs4::fs_std::FileExt;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_runtime::kernel_handle::KernelHandle;
use opencarrier_runtime::tool_runner::builtin_tool_definitions;
use opencarrier_types::agent::{AgentId, AgentIdentity, AgentManifest};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<OpenCarrierKernel>,
    pub started_at: Instant,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: opencarrier_runtime::provider_health::ProbeCache,
    /// Plugin manager (optional — only if plugins_dir is configured).
    #[allow(clippy::type_complexity)]
    pub plugin_manager: Option<Arc<tokio::sync::Mutex<opencarrier_runtime::plugin::PluginManager>>>,
}

// ---------------------------------------------------------------------------
// Helpers — shared patterns to reduce boilerplate in handlers
// ---------------------------------------------------------------------------

/// Parse a path-parameter agent ID, returning BAD_REQUEST on failure.
fn parse_agent_id(id: &str) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        )
    })
}

/// Parse agent ID, look up agent, and check tenant ownership.
/// Returns just the AgentId on success (for handlers that don't need the entry).
fn parse_agent_id_with_tenant(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    let (agent_id, _entry) = parse_and_get_agent_with_tenant(id, registry, ctx)?;
    Ok(agent_id)
}

/// Look up an agent in the registry, returning NOT_FOUND if missing.
fn get_agent_or_404(
    registry: &opencarrier_kernel::registry::AgentRegistry,
    agent_id: &AgentId,
) -> Result<opencarrier_types::agent::AgentEntry, (StatusCode, Json<serde_json::Value>)> {
    registry.get(*agent_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        )
    })
}

/// Parse agent ID from path and look up the agent. Returns (AgentId, AgentEntry) or an error response.
fn parse_and_get_agent(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
) -> Result<(AgentId, opencarrier_types::agent::AgentEntry), (StatusCode, Json<serde_json::Value>)> {
    let agent_id = parse_agent_id(id)?;
    let entry = get_agent_or_404(registry, &agent_id)?;
    Ok((agent_id, entry))
}

/// Parse agent ID, get agent entry, and check tenant ownership.
/// Returns 403 if the requester doesn't own the agent.
fn parse_and_get_agent_with_tenant(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<(AgentId, opencarrier_types::agent::AgentEntry), (StatusCode, Json<serde_json::Value>)> {
    let (agent_id, entry) = parse_and_get_agent(id, registry)?;
    if !can_access(ctx, entry.tenant_id.as_deref()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
        ));
    }
    Ok((agent_id, entry))
}

/// Look up a clone by name and extract its workspace path.
/// Returns (AgentEntry, PathBuf) or an error response.
fn get_clone_workspace(
    name: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
) -> Result<(opencarrier_types::agent::AgentEntry, std::path::PathBuf), (StatusCode, Json<serde_json::Value>)> {
    let entry = registry.find_by_name(name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
        )
    })?;
    let workspace = entry.manifest.workspace.clone().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Agent has no workspace"})),
        )
    })?;
    Ok((entry, workspace))
}

/// Look up a clone by name, check tenant ownership, and extract its workspace path.
fn get_clone_workspace_with_tenant(
    name: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<(opencarrier_types::agent::AgentEntry, std::path::PathBuf), (StatusCode, Json<serde_json::Value>)> {
    let (entry, workspace) = get_clone_workspace(name, registry)?;
    if !can_access(ctx, entry.tenant_id.as_deref()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
        ));
    }
    Ok((entry, workspace))
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// POST /api/agents — Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    // Resolve template name → manifest_toml if template is provided and manifest_toml is empty
    let manifest_toml = if req.manifest_toml.trim().is_empty() {
        if let Some(ref tmpl_name) = req.template {
            // Sanitize template name to prevent path traversal
            let safe_name = tmpl_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>();
            if safe_name.is_empty() || safe_name != *tmpl_name {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid template name"})),
                );
            }
            let tmpl_path = state
                .kernel
                .config
                .home_dir
                .join("agents")
                .join(&safe_name)
                .join("agent.toml");
            match std::fs::read_to_string(&tmpl_path) {
                Ok(content) => content,
                Err(_) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(
                            serde_json::json!({"error": format!("Template '{}' not found", safe_name)}),
                        ),
                    );
                }
            }
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "Either 'manifest_toml' or 'template' is required"}),
                ),
            );
        }
    } else {
        req.manifest_toml.clone()
    };

    // SECURITY: Reject oversized manifests to prevent parser memory exhaustion.
    const MAX_MANIFEST_SIZE: usize = 1024 * 1024; // 1MB
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Manifest too large (max 1MB)"})),
        );
    }

    // SECURITY: Verify Ed25519 signature when a signed manifest is provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                // Ensure the signed manifest matches the provided manifest_toml
                if verified_toml.trim() != manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({"error": "Signed manifest content does not match manifest_toml"}),
                        ),
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit_log.record(
                    "system",
                    opencarrier_runtime::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "Manifest signature verification failed"})),
                );
            }
        }
    }

    let manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Invalid manifest TOML: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid manifest format"})),
            );
        }
    };

    let name = manifest.name.clone();
    match state.kernel.spawn_agent(manifest) {
        Ok(id) => {
            // Assign tenant ownership from the request context
            if ctx.tenant_id.is_some() {
                state.kernel.registry.set_tenant_id(id, ctx.tenant_id.clone());
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!(SpawnResponse {
                    agent_id: id.to_string(),
                    name,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Agent spawn failed"})),
            )
        }
    }
}

/// GET /api/agents — List all agents.
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let all_agents = if ctx.is_admin() {
        state.kernel.registry.list()
    } else if let Some(ref tid) = ctx.tenant_id {
        state.kernel.registry.list_by_tenant(tid)
    } else {
        vec![]
    };

    let agents: Vec<serde_json::Value> = all_agents
        .into_iter()
        .map(|e| {
            let (modality, model) = state.kernel.resolve_model_label(&e.manifest.model.modality);
            let ready = matches!(e.state, opencarrier_types::agent::AgentState::Running);

            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "last_active": e.last_active.to_rfc3339(),
                "modality": modality,
                "model": model,
                "ready": ready,
                "profile": e.manifest.profile,
                "identity": {
                    "emoji": e.identity.emoji,
                    "avatar_url": e.identity.avatar_url,
                    "color": e.identity.color,
                },
            })
        })
        .collect();

    Json(agents)
}

/// Resolve uploaded file attachments into ContentBlock::Image blocks.
///
/// Reads each file from the upload directory, base64-encodes it, and
/// returns image content blocks ready to insert into a session message.
pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<opencarrier_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("opencarrier_uploads");
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let content_type = if let Some(ref m) = meta {
            m.content_type.clone()
        } else if !att.content_type.is_empty() {
            att.content_type.clone()
        } else {
            continue; // Skip unknown attachments
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            continue;
        }

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);
        match std::fs::read(&file_path) {
            Ok(data) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(opencarrier_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data: b64,
                });
            }
            Err(e) => {
                tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read upload for attachment");
            }
        }
    }

    blocks
}

/// Pre-insert image attachments into an agent's session so the LLM can see them.
///
/// This injects image content blocks into the session BEFORE the kernel
/// adds the text user message, so the LLM receives: [..., User(images), User(text)].
pub fn inject_attachments_into_session(
    kernel: &OpenCarrierKernel,
    agent_id: AgentId,
    image_blocks: Vec<opencarrier_types::message::ContentBlock>,
) {
    use opencarrier_types::message::{Message, MessageContent, Role};

    let entry = match kernel.registry.get(agent_id) {
        Some(e) => e,
        None => return,
    };

    let mut session = match kernel.memory.get_session(entry.session_id) {
        Ok(Some(s)) => s,
        _ => opencarrier_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
            tenant_id: None,
        },
    };

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(image_blocks),
    });

    if let Err(e) = kernel.memory.save_session(&session) {
        tracing::warn!(error = %e, "Failed to save session with image attachments");
    }
}

/// POST /api/agents/:id/message — Send a message to an agent.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    extensions: axum::http::Extensions,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok((aid, _)) => aid,
        Err(resp) => return resp,
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    // Resolve file attachments into image content blocks
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(
            agent_id,
            &req.message,
            Some(kernel_handle),
            req.sender_id,
            req.sender_name,
        )
        .await
    {
        Ok(result) => {
            // Strip <think>...</think> blocks from model output
            let cleaned = crate::ws::strip_think_tags(&result.response);

            // If the agent intentionally returned a silent/NO_REPLY response,
            // return an empty string — don't generate debug fallback text.
            let response = if result.silent {
                String::new()
            } else if cleaned.trim().is_empty() {
                format!(
                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                cleaned
            };
            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            let status = if format!("{e}").contains("Agent not found") {
                StatusCode::NOT_FOUND
            } else if format!("{e}").contains("quota") || format!("{e}").contains("Quota") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
            )
        }
    }
}

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

/// DELETE /api/agents/:id — Kill an agent.
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok((aid, _)) => aid,
        Err(resp) => return resp,
    };

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        ),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found or already terminated"})),
            )
        }
    }
}

/// POST /api/agents/{id}/restart — Restart a crashed/stuck agent.
///
/// Cancels any active task, resets agent state to Running, and updates last_active.
/// Returns the agent's new state.
pub async fn restart_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let agent_name = entry.name.clone();
    let previous_state = format!("{:?}", entry.state);
    drop(entry);

    // Cancel any running task
    let was_running = state.kernel.stop_agent_run(agent_id).unwrap_or(false);

    // Reset state to Running (also updates last_active)
    let _ = state
        .kernel
        .registry
        .set_state(agent_id, opencarrier_types::agent::AgentState::Running);

    tracing::info!(
        agent = %agent_name,
        previous_state = %previous_state,
        task_cancelled = was_running,
        "Agent restarted via API"
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "restarted",
            "agent": agent_name,
            "agent_id": id,
            "previous_state": previous_state,
            "task_cancelled": was_running,
        })),
    )
}

/// GET /api/status — Kernel status.
pub async fn status(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents_owned = if ctx.is_admin() {
        all_agents
    } else {
        all_agents.into_iter().filter(|e| can_access(&ctx, e.tenant_id.as_deref())).collect()
    };
    let agents: Vec<serde_json::Value> = agents_owned
        .into_iter()
        .map(|e| {
            let (modality, model) = state.kernel.resolve_model_label(&e.manifest.model.modality);
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "modality": modality,
                "model": model,
                "profile": e.manifest.profile,
            })
        })
        .collect();

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();
    let (default_modality, default_model) = state.kernel.resolve_model_label("chat");

    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "default_modality": default_modality,
        "default_model": default_model,
        "uptime_seconds": uptime,
        "api_listen": state.kernel.config.api_listen,
        "home_dir": state.kernel.config.home_dir.display().to_string(),
        "log_level": state.kernel.config.log_level,
        "agents": agents,
    }))
}

// ── Provider API Key management ────────────────────────────────────────────

/// GET /api/providers/keys — List all providers with API key status.
pub async fn list_provider_keys(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let brain = state.kernel.brain_info();
    let config = brain.config();

    let providers: Vec<serde_json::Value> = config
        .providers
        .iter()
        .map(|(name, pc)| {
            let endpoints: Vec<String> = config
                .endpoints
                .values()
                .filter(|ep| ep.provider == *name)
                .map(|ep| ep.model.clone())
                .collect();

            let has_key = if pc.auth_type == "jwt" {
                // JWT auth: check that all param env vars are set
                pc.params.values().all(|env_name| opencarrier_kernel::dotenv::has_env_key(env_name))
            } else {
                opencarrier_kernel::dotenv::has_env_key(&pc.api_key_env)
            };

            let params_status: Vec<serde_json::Value> = pc.params.iter()
                .map(|(logical_name, env_name)| {
                    serde_json::json!({
                        "name": logical_name,
                        "env": env_name,
                        "has_value": opencarrier_kernel::dotenv::has_env_key(env_name),
                    })
                })
                .collect();

            serde_json::json!({
                "name": name,
                "api_key_env": pc.api_key_env,
                "auth_type": pc.auth_type,
                "has_key": has_key,
                "params": params_status,
                "endpoints": endpoints,
            })
        })
        .collect();

    Json(serde_json::json!({ "providers": providers }))
}

/// POST /api/providers/{name}/key — Set API key for a provider.
///
/// For `apikey` auth type: `{ "key": "sk-xxx" }`
/// For `jwt` auth type: `{ "params": { "access_key_env": "val", "secret_key_env": "val" } }`
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let brain = state.kernel.brain_info();
    let config = brain.config();

    let pc = match config.providers.get(&name) {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Provider '{}' not found", name)})),
            );
        }
    };

    match pc.auth_type.as_str() {
        "jwt" => {
            // JWT auth: save each param value to its corresponding env var
            let params = match body.get("params").and_then(|p| p.as_object()) {
                Some(p) => p,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "JWT provider requires 'params' object"})),
                    );
                }
            };

            for (logical_name, env_name) in &pc.params {
                if let Some(val) = params.get(logical_name).and_then(|v| v.as_str()) {
                    let val = val.trim();
                    if val.is_empty() {
                        continue;
                    }
                    if let Err(e) = opencarrier_kernel::dotenv::save_env_key(env_name, val) {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": e})),
                        );
                    }
                }
            }
        }
        _ => {
            // Default apikey auth
            let key = body["key"].as_str().unwrap_or("").trim();
            if key.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Missing 'key' field"})),
                );
            }
            if let Err(e) = opencarrier_kernel::dotenv::save_env_key(&pc.api_key_env, key) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e})),
                );
            }
        }
    }

    let reload_result = state.kernel.reload_brain();
    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        format!("API key set for provider '{}'", name),
        if reload_result.is_ok() { "ok" } else { "reload_failed" },
    );
    match reload_result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok"})),
        ),
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "warning": format!("Key saved but brain reload failed: {}", e)})),
        ),
    }
}

/// DELETE /api/providers/{name}/key — Remove API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let brain = state.kernel.brain_info();
    let config = brain.config();

    let pc = match config.providers.get(&name) {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Provider '{}' not found", name)})),
            );
        }
    };

    if pc.auth_type == "jwt" {
        // Delete all param env vars
        for env_name in pc.params.values() {
            let _ = opencarrier_kernel::dotenv::delete_env_key(env_name);
        }
    } else {
        let _ = opencarrier_kernel::dotenv::delete_env_key(&pc.api_key_env);
    }

    let _ = state.kernel.reload_brain();
    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        format!("API key removed for provider '{}'", name),
        "ok",
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok"})),
    )
}

/// GET /api/brain — Brain configuration and status.
///
/// Returns the Brain's modalities, endpoints, and which ones are ready.
pub async fn brain_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let brain = state.kernel.brain_info();
    let config = brain.config();
    let ready = brain.ready_endpoints();

    let mut endpoints = serde_json::Map::new();
    for (name, ep) in &config.endpoints {
        endpoints.insert(name.clone(), serde_json::json!({
            "provider": ep.provider,
            "model": ep.model,
            "base_url": ep.base_url,
            "format": ep.format.to_string(),
            "ready": ready.contains(&name.as_str()),
        }));
    }

    let mut modalities = serde_json::Map::new();
    for (name, mc) in &config.modalities {
        modalities.insert(name.clone(), serde_json::json!({
            "primary": mc.primary,
            "fallbacks": mc.fallbacks,
        }));
    }

    Json(serde_json::json!({
        "loaded": true,
        "default_modality": config.default_modality,
        "modalities": modalities,
        "endpoints": endpoints,
    }))
}

/// GET /api/brain/status — Brain health status (driver readiness, latency, success/failure).
pub async fn brain_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let brain = state.kernel.brain_info();
    let status = brain.status();
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// GET /api/brain/modalities/{name} — Resolved endpoint chain for a single modality.
pub async fn brain_modality_detail(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let brain = state.kernel.brain_info();
    let endpoints = brain.endpoints_for(&name);
    if endpoints.is_empty() {
        // Check if modality exists at all
        let config = brain.config();
        if !config.modalities.contains_key(&name) {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Modality '{}' not found", name)})),
            );
        }
    }
    let config = brain.config();
    let mc = config.modalities.get(&name);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "modality": name,
            "description": mc.map(|m| m.description.clone()).unwrap_or_default(),
            "primary": mc.map(|m| m.primary.clone()).unwrap_or_default(),
            "fallbacks": mc.map(|m| m.fallbacks.clone()).unwrap_or_default(),
            "endpoints": endpoints,
        })),
    )
}

// ── Brain config management ────────────────────────────────────────────────

/// PUT /api/brain/providers/{name} — create or update a Brain provider.
pub async fn set_brain_provider(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let api_key_env = body["api_key_env"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    let result = state.kernel.update_brain(|config| {
        config.providers.insert(
            name.clone(),
            opencarrier_types::brain::ProviderConfig { api_key_env, auth_type: "apikey".to_string(), params: std::collections::HashMap::new() },
        );
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain provider '{name}' updated"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "provider": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/brain/providers/{name} — remove a Brain provider.
pub async fn delete_brain_provider(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    // Check no endpoints reference this provider
    let guard = state.kernel.brain_read();
    {
        let refs: Vec<String> = guard
            .config()
            .endpoints
            .iter()
            .filter(|(_, ep)| ep.provider == name)
            .map(|(n, _)| n.clone())
            .collect();
        if !refs.is_empty() {
            drop(guard);
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!("Provider '{name}' is used by endpoints: {}", refs.join(", "))
                })),
            );
        }
    }
    drop(guard);

    let result = state.kernel.update_brain(|config| {
        config.providers.remove(&name);
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain provider '{name}' deleted"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "deleted": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// PUT /api/brain/endpoints/{name} — create or update a Brain endpoint.
pub async fn set_brain_endpoint(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let provider = body["provider"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let model = body["model"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let base_url = body["base_url"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let format_str = body["format"]
        .as_str()
        .unwrap_or("openai")
        .trim()
        .to_string();

    // Validate required fields
    if provider.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'provider' field"})),
        );
    }
    if model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'model' field"})),
        );
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base_url must start with http:// or https://"})),
        );
    }

    let format = match format_str.as_str() {
        "openai" => opencarrier_types::brain::ApiFormat::OpenAI,
        "anthropic" => opencarrier_types::brain::ApiFormat::Anthropic,
        "gemini" => opencarrier_types::brain::ApiFormat::Gemini,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "format must be 'openai', 'anthropic', or 'gemini'"})),
            )
        }
    };

    // Validate provider exists
    {
        let guard = state.kernel.brain_read();
        if !guard.config().providers.contains_key(&provider) {
            drop(guard);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Provider '{provider}' not found")})),
            );
        }
    }

    let result = state.kernel.update_brain(|config| {
        config.endpoints.insert(
            name.clone(),
            opencarrier_types::brain::EndpointConfig {
                provider,
                model,
                base_url,
                format,
                auth_header: Default::default(),
            },
        );
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain endpoint '{name}' updated"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "endpoint": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/brain/endpoints/{name} — remove a Brain endpoint.
pub async fn delete_brain_endpoint(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    // Check no modalities reference this endpoint
    let guard = state.kernel.brain_read();
    {
        let refs: Vec<String> = guard
            .config()
            .modalities
            .iter()
            .filter(|(_, mc)| mc.primary == name || mc.fallbacks.contains(&name))
            .map(|(n, _)| n.clone())
            .collect();
        if !refs.is_empty() {
            drop(guard);
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!("Endpoint '{name}' is used by modalities: {}", refs.join(", "))
                })),
            );
        }
    }
    drop(guard);

    let result = state.kernel.update_brain(|config| {
        config.endpoints.remove(&name);
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain endpoint '{name}' deleted"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "deleted": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// PUT /api/brain/modalities/{name} — create or update a Brain modality.
pub async fn set_brain_modality(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let primary = body["primary"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let fallbacks: Vec<String> = body["fallbacks"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if primary.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'primary' field"})),
        );
    }

    // Validate endpoints exist
    let guard = state.kernel.brain_read();
    if !guard.config().endpoints.contains_key(&primary) {
        drop(guard);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Primary endpoint '{primary}' not found")})),
        );
    }
    for fb in &fallbacks {
        if !guard.config().endpoints.contains_key(fb) {
            drop(guard);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Fallback endpoint '{fb}' not found")})),
            );
        }
    }
    drop(guard);

    let result = state.kernel.update_brain(|config| {
        config.modalities.insert(
            name.clone(),
            opencarrier_types::brain::ModalityConfig {
                primary,
                fallbacks,
                description: String::new(),
            },
        );
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain modality '{name}' updated"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "modality": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/brain/modalities/{name} — remove a Brain modality.
pub async fn delete_brain_modality(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    // Cannot delete default modality
    let guard = state.kernel.brain_read();
    if guard.config().default_modality == name {
        drop(guard);
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Cannot delete default modality '{name}'")})),
        );
    }
    drop(guard);

    let result = state.kernel.update_brain(|config| {
        config.modalities.remove(&name);
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("brain modality '{name}' deleted"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "deleted": name})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// PUT /api/brain/default-modality — set the default modality.
pub async fn set_brain_default_modality(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let modality = body["default_modality"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    if modality.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'default_modality' field"})),
        );
    }

    let result = state.kernel.update_brain(|config| {
        if !config.modalities.contains_key(&modality) {
            return;
        }
        config.default_modality = modality.clone();
    });

    match result {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                format!("default modality set to '{modality}'"),
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "default_modality": modality})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// POST /api/brain/reload — reload Brain from disk.
pub async fn reload_brain(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    match state.kernel.reload_brain() {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                "brain reloaded from disk",
                "ok",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Brain reloaded"})),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// GET /api/brain/config — Return raw brain.json content.
pub async fn get_brain_config_raw(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let path = state.kernel.brain_path();
    match std::fs::read_to_string(path) {
        Ok(json_str) => match serde_json::from_str::<serde_json::Value>(&json_str) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(_) => (
                StatusCode::OK,
                Json(serde_json::json!({"_raw": json_str})),
            ),
        },
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Cannot read brain.json: {e}")})),
        ),
    }
}

/// PUT /api/brain/config — Update brain.json from raw JSON.
pub async fn put_brain_config_raw(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    // Validate it's a valid BrainConfig before writing
    let config: opencarrier_types::brain::BrainConfig = match serde_json::from_value(body.clone()) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid brain config: {e}")})),
            );
        }
    };

    let path = state.kernel.brain_path();
    let json_str = match serde_json::to_string_pretty(&config) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Serialize error: {e}")})),
            );
        }
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, json_str) {
        Ok(()) => {
            let reload_result = state.kernel.reload_brain();
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::ConfigChange,
                "brain.json updated via API",
                if reload_result.is_ok() { "ok" } else { "reload_failed" },
            );
            match reload_result {
                Ok(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "ok"})),
                ),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Saved but reload failed: {e}")})),
                ),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Write error: {e}")})),
        ),
    }
}

/// POST /api/shutdown — Graceful shutdown.
pub async fn shutdown(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    tracing::info!("Shutdown requested via API");
    // SECURITY: Record shutdown in audit trail
    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    // Signal the HTTP server to initiate graceful shutdown so the process exits.
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"})).into_response()
}

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
pub async fn list_profiles() -> impl IntoResponse {
    use opencarrier_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// PUT /api/agents/:id/mode — Change an agent's operational mode.
pub async fn set_agent_mode(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match state.kernel.registry.set_mode(agent_id, body.mode) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "agent_id": id,
                "mode": body.mode,
            })),
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Version endpoint
// ---------------------------------------------------------------------------

/// GET /api/version — Build & version info.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "opencarrier",
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

// ---------------------------------------------------------------------------
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------

/// GET /api/agents/:id — Get a single agent's detailed info.
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "modality": entry.manifest.model.modality,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "skills": entry.manifest.skills,
            "skills_mode": if entry.manifest.skills.is_empty() { "all" } else { "allowlist" },
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
        })),
    )
}

/// POST /api/agents/:id/message/stream — SSE streaming response.
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use opencarrier_runtime::llm_driver::StreamEvent;

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        )
            .into_response();
    }

    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok((aid, _)) => aid,
        Err(resp) => return resp.into_response(),
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle) = match state.kernel.send_message_streaming(
        agent_id,
        &req.message,
        Some(kernel_handle),
        req.sender_id,
        req.sender_name,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("Streaming message failed for agent {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Streaming message failed"})),
            )
                .into_response();
        }
    };

    let sse_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => Event::default()
                        .event("chunk")
                        .json_data(serde_json::json!({"content": text, "done": false}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseStart { name, .. } => Event::default()
                        .event("tool_use")
                        .json_data(serde_json::json!({"tool": name}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseEnd { name, input, .. } => Event::default()
                        .event("tool_result")
                        .json_data(serde_json::json!({"tool": name, "input": input}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ContentComplete { usage, .. } => Event::default()
                        .event("done")
                        .json_data(serde_json::json!({
                            "done": true,
                            "usage": {
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                            }
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::PhaseChange { phase, detail } => Event::default()
                        .event("phase")
                        .json_data(serde_json::json!({
                            "phase": phase,
                            "detail": detail,
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    _ => Event::default().comment("skip"),
                });
                Some((sse_event, rx))
            }
            None => None,
        }
    });

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
pub async fn list_templates() -> impl IntoResponse {
    let agents_dir = opencarrier_kernel::config::opencarrier_home().join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("agent.toml");
                if manifest_path.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let description = std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                        .map(|m| m.description)
                        .unwrap_or_default();

                    templates.push(serde_json::json!({
                        "name": name,
                        "description": description,
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates/:name — Get template details.
pub async fn get_template(Path(name): Path<String>) -> impl IntoResponse {
    // Reject path traversal attempts
    if name.contains('.') || name.contains(std::path::MAIN_SEPARATOR) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid template name"})),
        );
    }

    let agents_dir = opencarrier_kernel::config::opencarrier_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    // Verify resolved path stays within agents_dir
    if let (Ok(canonical_agents), Ok(canonical_manifest)) =
        (agents_dir.canonicalize(), manifest_path.canonicalize())
    {
        if !canonical_manifest.starts_with(&canonical_agents) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid template name"})),
            );
        }
    }

    if !manifest_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Template not found"})),
        );
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "modality": manifest.model.modality,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Invalid template manifest"})),
                )
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read template"})),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Memory endpoints
// ---------------------------------------------------------------------------

/// GET /api/memory/agents/:id/kv — List KV pairs for an agent.
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match state.kernel.memory.list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(e) => {
            tracing::warn!("Memory list_kv failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// GET /api/memory/agents/:id/kv/:key — Get a specific KV value.
pub async fn get_agent_kv_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match state.kernel.memory.structured_get(agent_id, &key) {
        Ok(Some(val)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": val})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Key not found"})),
        ),
        Err(e) => {
            tracing::warn!("Memory get failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// PUT /api/memory/agents/:id/kv/:key — Set a KV value.
pub async fn set_agent_kv_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, key)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let value = body.get("value").cloned().unwrap_or(body);

    match state.kernel.memory.structured_set(agent_id, &key, value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory set failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// DELETE /api/memory/agents/:id/kv/:key — Delete a KV value.
pub async fn delete_agent_kv_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match state.kernel.memory.structured_delete(agent_id, &key) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory delete failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// GET /api/health — Minimal liveness probe (public, no auth required).
/// Returns only status and version to prevent information leakage.
/// Use GET /api/health/detail for full diagnostics (requires auth).
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Run the database check on a blocking thread so we never hold the
    // std::sync::Mutex<Connection> on a tokio worker thread.  This prevents
    // the health probe from starving the async runtime when the agent loop
    // is holding the database lock for session saves.
    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || {
        let shared_id = opencarrier_types::agent::AgentId(uuid::Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]));
        memory.structured_get(shared_id, "__health_check__").is_ok()
    })
    .await
    .unwrap_or(false);

    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/health/detail — Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();

    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || {
        let shared_id = opencarrier_types::agent::AgentId(uuid::Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]));
        memory.structured_get(shared_id, "__health_check__").is_ok()
    })
    .await
    .unwrap_or(false);

    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "config_warnings": config_warnings,
    }))
}

// ---------------------------------------------------------------------------
// Prometheus metrics endpoint
// ---------------------------------------------------------------------------

/// GET /api/metrics — Prometheus text-format metrics.
///
/// Returns counters and gauges for monitoring OpenCarrier in production:
/// - `opencarrier_agents_active` — number of active agents
/// - `opencarrier_uptime_seconds` — seconds since daemon started
/// - `opencarrier_tokens_total` — total tokens consumed (per agent)
/// - `opencarrier_tool_calls_total` — total tool calls (per agent)
/// - `opencarrier_panics_total` — supervisor panic count
/// - `opencarrier_restarts_total` — supervisor restart count
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    let mut out = String::with_capacity(2048);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP opencarrier_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE opencarrier_uptime_seconds gauge\n");
    out.push_str(&format!("opencarrier_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, opencarrier_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP opencarrier_agents_active Number of active agents.\n");
    out.push_str("# TYPE opencarrier_agents_active gauge\n");
    out.push_str(&format!("opencarrier_agents_active {active}\n"));
    out.push_str("# HELP opencarrier_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE opencarrier_agents_total gauge\n");
    out.push_str(&format!("opencarrier_agents_total {}\n\n", agents.len()));

    // Per-agent token and tool usage
    out.push_str(
        "# HELP opencarrier_tokens_total Total tokens consumed (rolling hourly window).\n",
    );
    out.push_str("# TYPE opencarrier_tokens_total gauge\n");
    out.push_str("# HELP opencarrier_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE opencarrier_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let modality = &agent.manifest.model.modality;
        let model = &agent.manifest.model.modality;
        if let Some((tokens, tools)) = state.kernel.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "opencarrier_tokens_total{{agent=\"{name}\",modality=\"{modality}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "opencarrier_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.supervisor.health();
    out.push_str("# HELP opencarrier_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE opencarrier_panics_total counter\n");
    out.push_str(&format!(
        "opencarrier_panics_total {}\n",
        health.panic_count
    ));
    out.push_str("# HELP opencarrier_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE opencarrier_restarts_total counter\n");
    out.push_str(&format!(
        "opencarrier_restarts_total {}\n\n",
        health.restart_count
    ));

    // Version info
    out.push_str("# HELP opencarrier_info OpenCarrier version and build info.\n");
    out.push_str("# TYPE opencarrier_info gauge\n");
    out.push_str(&format!(
        "opencarrier_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    ).into_response()
}
// ---------------------------------------------------------------------------

/// GET /api/skills — List installed skills.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = opencarrier_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    let skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            let source = match &s.manifest.source {
                Some(opencarrier_skills::SkillSource::Hub { slug, version }) => {
                    serde_json::json!({"type": "hub", "slug": slug, "version": version})
                }
                Some(opencarrier_skills::SkillSource::Native) | None => {
                    serde_json::json!({"type": "local"})
                }
            };
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "version": s.manifest.skill.version,
                "author": s.manifest.skill.author,
                "runtime": format!("{:?}", s.manifest.runtime.runtime_type),
                "tools_count": s.manifest.tools.provided.len(),
                "tags": s.manifest.skill.tags,
                "enabled": s.enabled,
                "source": source,
                "has_prompt_context": s.manifest.prompt_context.is_some(),
            })
        })
        .collect();

    Json(serde_json::json!({ "skills": skills, "total": skills.len() }))
}

// ---------------------------------------------------------------------------
// MCP server endpoints
// ---------------------------------------------------------------------------

/// GET /api/mcp/servers — List configured MCP servers and their tools.
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Get configured servers from config
    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|s| {
            let transport = match &s.transport {
                opencarrier_types::config::McpTransportEntry::Stdio { command, args } => {
                    serde_json::json!({
                        "type": "stdio",
                        "command": command,
                        "args": args,
                    })
                }
                opencarrier_types::config::McpTransportEntry::Sse { url } => {
                    serde_json::json!({
                        "type": "sse",
                        "url": url,
                    })
                }
            };
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections
    let connections = state.kernel.mcp_connections.lock().await;
    let connected: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let tools: Vec<serde_json::Value> = conn
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect();
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": true,
            })
        })
        .collect();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": connected.len(),
    }))
}

// ---------------------------------------------------------------------------
// Audit endpoints
// ---------------------------------------------------------------------------

/// GET /api/audit/recent — Get recent audit log entries.
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000); // Cap at 1000

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    })).into_response()
}

/// GET /api/audit/verify — Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                })).into_response()
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                })).into_response()
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })).into_response(),
    }
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///   - `token`  — auth token (for EventSource clients that cannot set headers)
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second and sends only new entries
/// (tracked by sequence number). On first connect, existing entries are sent
/// as a backfill so the client has immediate context.
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = state.kernel.audit_log.recent(200);

            for entry in &entries {
                // On first poll, send all existing entries as backfill.
                // After that, only send entries newer than last_seq.
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}

// ---------------------------------------------------------------------------
// Tools endpoint
// ---------------------------------------------------------------------------

/// GET /api/tools — List all tool definitions (built-in + MCP).
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tools: Vec<serde_json::Value> = builtin_tool_definitions()
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    // Include MCP tools so they're visible in Settings -> Tools
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        for t in mcp_tools.iter() {
            tools.push(serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "source": "mcp",
            }));
        }
    }

    Json(serde_json::json!({"tools": tools, "total": tools.len()}))
}

// ---------------------------------------------------------------------------
// Config endpoint
// ---------------------------------------------------------------------------

/// GET /api/config — Get kernel configuration (secrets redacted).
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.kernel.config;
    let (default_modality, default_model) = state.kernel.resolve_model_label("chat");
    Json(serde_json::json!({
        "home_dir": config.home_dir.to_string_lossy(),
        "data_dir": config.data_dir.to_string_lossy(),
        "api_key": if config.api_key.is_empty() { "not set" } else { "***" },
        "brain": {
            "config_path": config.brain.config,
            "default_modality": default_modality,
            "default_model": default_model,
        },
        "memory": {
            "decay_rate": config.memory.decay_rate,
        },
    }))
}

// ---------------------------------------------------------------------------
// Usage endpoint
// ---------------------------------------------------------------------------

/// GET /api/usage — Get per-agent usage statistics.
pub async fn usage_stats(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents_filtered: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents.into_iter().filter(|e| can_access(&ctx, e.tenant_id.as_deref())).collect()
    };
    let agents: Vec<serde_json::Value> = agents_filtered
        .iter()
        .map(|e| {
            let (tokens, tool_calls) = state.kernel.scheduler.get_usage(e.id).unwrap_or((0, 0));
            serde_json::json!({
                "agent_id": e.id.to_string(),
                "name": e.name,
                "total_tokens": tokens,
                "tool_calls": tool_calls,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents}))
}

// ---------------------------------------------------------------------------
// Usage summary endpoints
// ---------------------------------------------------------------------------

/// GET /api/usage/summary — Get overall usage summary from UsageStore.
pub async fn usage_summary(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() { None } else { ctx.tenant_id.clone() };
    match state.kernel.memory.usage().query_summary(None, tid.as_deref()) {
        Ok(s) => Json(serde_json::json!({
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "call_count": s.call_count,
            "total_tool_calls": s.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}

/// GET /api/usage/by-model — Get usage grouped by model.
pub async fn usage_by_model(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() { None } else { ctx.tenant_id.clone() };
    match state.kernel.memory.usage().query_by_model(tid.as_deref()) {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_input_tokens": m.total_input_tokens,
                        "total_output_tokens": m.total_output_tokens,
                        "call_count": m.call_count,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}

/// GET /api/usage/daily — Get daily usage breakdown for the last 7 days.
pub async fn usage_daily(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() { None } else { ctx.tenant_id.clone() };
    let days = state.kernel.memory.usage().query_daily_breakdown(7, tid.as_deref());
    let first_event = state.kernel.memory.usage().query_first_event_date();

    let days_list = match days {
        Ok(d) => d
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "first_event_date": first_event.unwrap_or(None),
    }))
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => opencarrier_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

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
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => opencarrier_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

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
    Path((agent_id_str, label)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match agent_id_str.parse::<uuid::Uuid>() {
        Ok(u) => opencarrier_types::agent::AgentId(u),
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&agent_id_str) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Agent not found"})),
                    );
                }
            }
        }
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

// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/agents/:id — Update an agent (currently: re-set manifest fields).
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(_req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let _ = parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx);

    (
        StatusCode::GONE,
        Json(serde_json::json!({
            "error": "In-place manifest update is not supported. Use DELETE + POST (kill and respawn) to apply changes.",
            "agent_id": id,
        })),
    )
}

/// PATCH /api/agents/{id} — Partial update of agent fields (name, description, model, system_prompt).
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    // Apply partial updates using dedicated registry methods
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_name(agent_id, name.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_description(agent_id, desc.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .set_agent_model(agent_id, model)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }
    if let Some(system_prompt) = body.get("system_prompt").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_system_prompt(agent_id, system_prompt.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }

    // Persist updated entry to SQLite
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        let _ = state.kernel.memory.save_agent(&entry);
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "agent_id": entry.id.to_string(), "name": entry.name}),
            ),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Agent vanished during update"})),
        )
    }
}

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------

/// GET /api/security — Security feature status for the dashboard.
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = if state.kernel.config.api_key.is_empty() {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit_log.len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.is_empty()
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}

// ── MCP HTTP Endpoint ───────────────────────────────────────────────────

/// POST /mcp — Handle MCP JSON-RPC requests over HTTP.
///
/// Exposes the same MCP protocol normally served via stdio, allowing
/// external MCP clients to connect over HTTP instead.
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Gather all available tools (builtin + skills + MCP)
    let mut tools = builtin_tool_definitions();
    {
        let registry = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for skill_tool in registry.all_tool_definitions() {
            tools.push(opencarrier_types::tool::ToolDefinition {
                name: skill_tool.name.clone(),
                description: skill_tool.description.clone(),
                input_schema: skill_tool.input_schema.clone(),
            });
        }
    }
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        tools.extend(mcp_tools.iter().cloned());
    }

    // Check if this is a tools/call that needs real execution
    let method = request["method"].as_str().unwrap_or("");
    if method == "tools/call" {
        let tool_name = request["params"]["name"].as_str().unwrap_or("");
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Verify the tool exists
        if !tools.iter().any(|t| t.name == tool_name) {
            return Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned(),
                "error": {"code": -32602, "message": format!("Unknown tool: {tool_name}")}
            }));
        }

        // Snapshot skill registry before async call (RwLockReadGuard is !Send)
        let skill_snapshot = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        // Execute the tool via the kernel's tool runner
        let kernel_handle: Arc<dyn opencarrier_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn opencarrier_runtime::kernel_handle::KernelHandle>;
        let result = opencarrier_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            None,
            None,
            Some(&skill_snapshot),
            Some(&state.kernel.mcp_connections),
            Some(&state.kernel.web_ctx),
            Some(&state.kernel.browser_ctx),
            None,
            None,
            Some(&state.kernel.media_engine),
            Some(&state.kernel.config.exec_policy),
            if state.kernel.config.tts.enabled {
                Some(&state.kernel.tts_engine)
            } else {
                None
            },
            if state.kernel.config.docker.enabled {
                Some(&state.kernel.config.docker)
            } else {
                None
            },
            Some(&*state.kernel.process_manager),
            None, // sender_id (MCP HTTP calls have no sender context)
        )
        .await;

        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned(),
            "result": {
                "content": [{"type": "text", "text": result.content}],
                "isError": result.is_error,
            }
        }));
    }

    // For non-tools/call methods (initialize, tools/list, etc.), delegate to the handler
    let response = opencarrier_runtime::mcp_server::handle_mcp_request(&request, &tools).await;
    Json(response)
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

/// POST /api/agents/{id}/stop — Cancel an agent's current LLM run.
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// PUT /api/agents/{id}/model — Switch an agent's model.
pub async fn set_model(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'model' field"})),
            )
        }
    };
    let _provider_hint = body["provider"].as_str(); // Ignored — Brain manages providers
    match state
        .kernel
        .set_agent_model(agent_id, model)
    {
        Ok(()) => {
            // Return the resolved modality so frontend stays in sync.
            let resolved_modality = state
                .kernel
                .registry
                .get(agent_id)
                .map(|e| e.manifest.model.modality.clone())
                .unwrap_or_else(|| model.to_string());
            let (_, resolved_model) = state.kernel.resolve_model_label(&resolved_modality);
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "ok", "modality": resolved_modality, "model": resolved_model}),
                ),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/tools — Get an agent's tool allowlist/blocklist.
pub async fn get_agent_tools(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
        })),
    )
}

/// PUT /api/agents/{id}/tools — Update an agent's tool allowlist/blocklist.
pub async fn set_agent_tools(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let allowlist = body
        .get("tool_allowlist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let blocklist = body
        .get("tool_blocklist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

    if allowlist.is_none() && blocklist.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provide 'tool_allowlist' and/or 'tool_blocklist'"})),
        );
    }

    match state
        .kernel
        .set_agent_tool_filters(agent_id, allowlist, blocklist)
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ── Per-Agent Skill & MCP Endpoints ────────────────────────────────────

/// GET /api/agents/{id}/skills — Get an agent's skill assignment info.
pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let available = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    let mode = if entry.manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/skills — Update an agent's skill allowlist.
pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let skills: Vec<String> = body["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "skills": skills})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers — Get an agent's MCP server assignment info.
pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        let mut seen = std::collections::HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = opencarrier_runtime::mcp::extract_mcp_server(&tool.name) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = if entry.manifest.mcp_servers.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let servers: Vec<String> = body["mcp_servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/skills/create — Create a local prompt-only skill.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    let name = match body["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
            );
        }
    };

    // Validate name (alphanumeric + hyphens only)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Skill name must contain only letters, numbers, hyphens, and underscores"}),
            ),
        );
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    // Only allow prompt_only skills from the web UI for safety
    if runtime != "prompt_only" {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Only prompt_only skills can be created from the web UI"}),
            ),
        );
    }

    // Write skill.toml to ~/.opencarrier/skills/{name}/
    let skill_dir = state.kernel.config.home_dir.join("skills").join(&name);
    if skill_dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Skill '{}' already exists", name)})),
        );
    }

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create skill directory: {e}")})),
        );
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    if let Err(e) = std::fs::write(&toml_path, &toml_content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write skill.toml: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "name": name,
            "note": "Restart the daemon to load the new skill, or it will be available on next boot."
        })),
    )
}

// ---------------------------------------------------------------------------
// Agent Identity endpoint
// ---------------------------------------------------------------------------

/// Request body for updating agent visual identity.
#[derive(serde::Deserialize)]
pub struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/identity — Update an agent's visual identity.
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
            );
        }
    }

    let identity = AgentIdentity {
        emoji: req.emoji,
        avatar_url: req.avatar_url,
        color: req.color,
        archetype: req.archetype,
        vibe: req.vibe,
        greeting_style: req.greeting_style,
    };

    match state.kernel.registry.update_identity(agent_id, identity) {
        Ok(()) => {
            // Persist identity to SQLite
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "agent_id": id})),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Config Hot-Update
// ---------------------------------------------------------------------------

/// Request body for patching agent config (name, description, prompt, identity, modality).
#[derive(serde::Deserialize)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
    #[serde(alias = "modality")]
    pub model: Option<String>,
}

/// PATCH /api/agents/{id}/config — Hot-update agent name, description, system prompt, and identity.
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Input length limits
    const MAX_NAME_LEN: usize = 256;
    const MAX_DESC_LEN: usize = 4096;
    const MAX_PROMPT_LEN: usize = 65_536;

    if let Some(ref name) = req.name {
        if name.len() > MAX_NAME_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": format!("Name exceeds max length ({MAX_NAME_LEN} chars)")}),
                ),
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": format!("Description exceeds max length ({MAX_DESC_LEN} chars)")}),
                ),
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": format!("System prompt exceeds max length ({MAX_PROMPT_LEN} chars)")}),
                ),
            );
        }
    }

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
            );
        }
    }

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .registry
                .update_name(agent_id, new_name.clone())
            {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": format!("{e}")})),
                );
            }
        }
    }

    // Update description
    if let Some(ref new_desc) = req.description {
        if state
            .kernel
            .registry
            .update_description(agent_id, new_desc.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message)
    if let Some(ref new_prompt) = req.system_prompt {
        if state
            .kernel
            .registry
            .update_system_prompt(agent_id, new_prompt.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update identity fields (merge — only overwrite provided fields)
    let has_identity_field = req.emoji.is_some()
        || req.avatar_url.is_some()
        || req.color.is_some()
        || req.archetype.is_some()
        || req.vibe.is_some()
        || req.greeting_style.is_some();

    if has_identity_field {
        // Read current identity, merge with provided fields
        let current = state
            .kernel
            .registry
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = AgentIdentity {
            emoji: req.emoji.or(current.emoji),
            avatar_url: req.avatar_url.or(current.avatar_url),
            color: req.color.or(current.color),
            archetype: req.archetype.or(current.archetype),
            vibe: req.vibe.or(current.vibe),
            greeting_style: req.greeting_style.or(current.greeting_style),
        };
        if state
            .kernel
            .registry
            .update_identity(agent_id, merged)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update modality (Brain resolves the actual provider/model at inference time)
    if let Some(ref new_model) = req.model {
        if !new_model.is_empty()
            && state
                .kernel
                .registry
                .update_modality(agent_id, new_model.clone())
                .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Persist updated manifest to database so changes survive restart
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if let Err(e) = state.kernel.memory.save_agent(&entry) {
            tracing::warn!("Failed to persist agent config update: {e}");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
}

// ---------------------------------------------------------------------------
// Agent Cloning
// ---------------------------------------------------------------------------

/// Request body for cloning an agent.
#[derive(serde::Deserialize)]
pub struct CloneAgentRequest {
    pub new_name: String,
}

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    if req.new_name.len() > 256 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Name exceeds max length (256 chars)"})),
        );
    }

    if req.new_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_name cannot be empty"})),
        );
    }

    let source = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Spawn the cloned agent
    let new_id = match state.kernel.spawn_agent(cloned_manifest) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Clone spawn failed: {e}")})),
            );
        }
    };

    // Copy workspace files from source to destination
    let new_entry = state.kernel.registry.get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                for &fname in KNOWN_IDENTITY_FILES {
                    let src_file = src_can.join(fname);
                    let dst_file = dst_can.join(fname);
                    if src_file.exists() {
                        let _ = std::fs::copy(&src_file, &dst_file);
                    }
                }
            }
        }
    }

    // Copy identity from source
    let _ = state
        .kernel
        .registry
        .update_identity(new_id, source.identity.clone());

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
}

// ---------------------------------------------------------------------------
// Workspace File Editor endpoints
// ---------------------------------------------------------------------------

/// Whitelisted workspace identity files that can be read/written via API.
/// Immutable identity files — can be created but never overwritten via the API.
const IMMUTABLE_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
];

const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// GET /api/agents/{id}/files — List workspace identity files.
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        let path = workspace.join(name);
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
///
/// Immutable files (SOUL.md) cannot be overwritten once created.
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, filename)): Path<(String, String)>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    // Immutable files: cannot be overwritten once created
    if IMMUTABLE_IDENTITY_FILES.contains(&filename.as_str()) {
        let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };
        if let Some(ref workspace) = entry.manifest.workspace {
            let file_path = workspace.join(&*filename);
            if file_path.exists() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": format!("{} is immutable — it cannot be overwritten after creation. \
                        This file defines the clone's identity and must not be tampered with.", filename)
                    })),
                );
            }
        }
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "File content too large (max 32KB)"})),
        );
    }

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };

    let file_path = workspace.join(&filename);
    // For new files, check the parent directory instead
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        // Parent must be inside workspace
        file_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = workspace.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Write failed: {e}")})),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Rename failed: {e}")})),
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}

// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------

/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

/// Metadata stored alongside uploaded files.
struct UploadMeta {
    content_type: String,
}

/// In-memory upload metadata registry.
static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Allowed content type prefixes for upload.
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/", "text/", "application/pdf", "audio/"];

fn is_allowed_content_type(ct: &str) -> bool {
    ALLOWED_CONTENT_TYPES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
}

/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts raw body bytes. The client must set:
/// - `Content-Type` header (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `X-Filename` header (original filename)
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    // Validate agent ID format and tenant ownership
    let _agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Extract content type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf"}),
            ),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Validate size
    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": format!("File too large (max {} MB)", MAX_UPLOAD_SIZE / (1024 * 1024))}),
            ),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty file body"})),
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("opencarrier_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to create upload directory"})),
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save file"})),
        );
    }

    let size = body.len();
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            content_type: content_type.clone(),
        },
    );

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = opencarrier_types::media::MediaAttachment {
            media_type: opencarrier_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: opencarrier_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state
            .kernel
            .media_engine
            .transcribe_audio(&attachment)
            .await
        {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}

/// GET /api/uploads/{file_id} — Serve an uploaded file.
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let file_path = std::env::temp_dir()
        .join("opencarrier_uploads")
        .join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => m.content_type.clone(),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            "image/png".to_string()
        }
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approval_manager
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Config Reload endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/reload — Reload configuration from disk and apply hot-reloadable changes.
///
/// Reads the config file, diffs against current config, validates the new config,
/// and applies hot-reloadable actions (approval policy, cron limits, etc.).
/// Returns the reload plan showing what changed and what was applied.
pub async fn config_reload(State(state): State<Arc<AppState>>, extensions: axum::http::Extensions) -> impl IntoResponse {
    { let ctx = get_tenant_ctx(&extensions); if !ctx.is_admin() { return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))); } }
    // SECURITY: Record config reload in audit trail
    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partial"
            } else if plan.has_changes() {
                "applied"
            } else {
                "no_changes"
            };

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    "restart_required": plan.restart_required,
                    "restart_reasons": plan.restart_reasons,
                    "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                    "noop_changes": plan.noop_changes,
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Config Schema endpoint
// ---------------------------------------------------------------------------

/// GET /api/config/schema — Return a simplified JSON description of the config structure.
pub async fn config_schema(State(state): State<Arc<AppState>>, _extensions: axum::http::Extensions) -> impl IntoResponse {
    // NOTE: admin check requires middleware-level enforcement (mixed return types)
    // Build modality options from Brain config (or legacy model catalog)
    let modalities: Vec<String> = state
        .kernel
        .brain_info()
        .config()
        .modalities
        .keys()
        .cloned()
        .collect();

    Json(serde_json::json!({
        "sections": {
            "general": {
                "root_level": true,
                "fields": {
                    "api_listen": "string",
                    "api_key": "string",
                    "log_level": "string"
                }
            },
            "brain": {
                "hot_reloadable": true,
                "fields": {
                    "config": "string",
                    "default_modality": { "type": "select", "options": modalities }
                }
            },
            "memory": {
                "fields": {
                    "decay_rate": "number",
                    "vector_dims": "number"
                }
            },
            "web": {
                "fields": {
                    "provider": "string",
                    "timeout_secs": "number",
                    "max_results": "number"
                }
            },
            "browser": {
                "fields": {
                    "headless": "boolean",
                    "timeout_secs": "number",
                    "executable_path": "string"
                }
            },
            "network": {
                "fields": {
                    "enabled": "boolean",
                    "listen_addr": "string",
                    "shared_secret": "string"
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Config Set endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/set — Set a single config value and persist to config.toml.
///
/// Accepts JSON `{ "path": "section.key", "value": "..." }`.
/// Writes the value to the TOML config file and triggers a reload.
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"}))).into_response();
    }
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'path' field"})),
            ).into_response();
        }
    };
    let value = match body.get("value") {
        Some(v) => v.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'value' field"})),
            ).into_response();
        }
    };

    // Block sensitive keys that should not be changed via API
    const BLOCKED_KEYS: &[&str] = &[
        "api_key",
        "auth",
        "exec_policy",
        "docker",
    ];
    let lower = path.to_lowercase();
    for blocked in BLOCKED_KEYS {
        if lower.starts_with(blocked) || lower.contains(&format!(".{blocked}")) {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "error",
                    "error": format!("Cannot modify '{blocked}' via API — edit config.toml directly")
                })),
            ).into_response();
        }
    }

    let config_path = state.kernel.config.home_dir.join("config.toml");

    // Read existing config as a TOML table, or start fresh
    let mut table: toml::value::Table = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => toml::value::Table::new(),
        }
    } else {
        toml::value::Table::new()
    };

    // Convert JSON value to TOML value
    let toml_val = json_to_toml_value(&value);

    // Parse "section.key" path and set value
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            table.insert(parts[0].to_string(), toml_val);
        }
        2 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                t.insert(parts[1].to_string(), toml_val);
            }
        }
        3 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                let sub = t
                    .entry(parts[1].to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut t2) = sub {
                    t2.insert(parts[2].to_string(), toml_val);
                }
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"status": "error", "error": "path too deep (max 3 levels)"}),
                ),
            ).into_response();
        }
    }

    // Write back
    let toml_string = match toml::to_string_pretty(&table) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"status": "error", "error": format!("serialize failed: {e}")}),
                ),
            ).into_response();
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        ).into_response();
    }

    // Trigger reload
    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": reload_status, "path": path})),
    ).into_response()
}

/// Convert a serde_json::Value to a toml::Value.
fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                toml::Value::Integer(i as i64)
            } else if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        _ => toml::Value::String(value.to_string()),
    }
}


// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                let jobs = state.kernel.cron_scheduler.list_jobs(aid);
                if ctx.is_admin() {
                    jobs
                } else {
                    jobs.into_iter().filter(|j| can_access(&ctx, j.tenant_id.as_deref())).collect()
                }
            }
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid agent_id"})),
                );
            }
        }
    } else if ctx.is_admin() {
        state.kernel.cron_scheduler.list_all_jobs()
    } else {
        state.kernel.cron_scheduler.list_all_jobs_by_tenant(ctx.tenant_id.as_deref().unwrap_or(""))
    };
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}

/// POST /api/cron/jobs — Create a new cron job.
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    // Verify tenant owns the target agent
    if let Ok(aid) = agent_id.parse::<AgentId>() {
        if let Some(entry) = state.kernel.registry.get(aid) {
            if !can_access(&ctx, entry.tenant_id.as_deref()) {
                return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
            }
        }
    }
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"result": result})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.remove_job(job_id) {
                Ok(_) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "deleted"})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.set_enabled(job_id, enabled) {
                Ok(()) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Job not found"})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<opencarrier_types::webhook::WakePayload>,
) -> impl IntoResponse {
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    // Validate bearer token (constant-time comparison)
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    // Publish through the kernel's publish_event (KernelHandle trait), which
    // goes through the full event processing pipeline including trigger evaluation.
    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Event publish failed: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<opencarrier_types::webhook::AgentHookPayload>,
) -> impl IntoResponse {
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    // Validate bearer token
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.registry.find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(
                                serde_json::json!({"error": format!("Agent not found: {}", agent_ref)}),
                            ),
                        );
                    }
                }
            }
        },
        None => {
            // SECURITY: No default agent — must specify explicitly
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Must specify 'agent' field (name or UUID)"})),
            );
        }
    };

    // Actually send the message to the agent and get the response
    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Agent execution failed: {e}")})),
        ),
    }
}

// ─── Agent Bindings API ────────────────────────────────────────────────

/// GET /api/bindings — List all agent bindings.
pub async fn list_bindings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bindings = state.kernel.list_bindings();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": bindings })),
    )
}

/// POST /api/bindings — Add a new agent binding.
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    Json(binding): Json<opencarrier_types::config::AgentBinding>,
) -> impl IntoResponse {
    // Validate agent exists
    let agents = state.kernel.registry.list();
    let agent_exists = agents.iter().any(|e| e.name == binding.agent)
        || binding.agent.parse::<uuid::Uuid>().is_ok();
    if !agent_exists {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    state.kernel.add_binding(binding);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created" })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    match state.kernel.remove_binding(index) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Binding index out of range" })),
        ),
    }
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Reset session (clear history)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ];

    // Add skill-registered tool names as potential commands
    if let Ok(registry) = state.kernel.skill_registry.read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// SECURITY: Validate webhook bearer token using constant-time comparison.
fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(t) if t.len() >= 32 => t,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) if s.starts_with("Bearer ") => &s[7..],
            _ => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Agent Communication (Comms) endpoints
// ---------------------------------------------------------------------------

/// GET /api/comms/topology — Build agent topology graph from registry.
pub async fn comms_topology(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    use opencarrier_types::comms::{EdgeKind, TopoEdge, TopoNode, Topology};

    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|a| can_access(&ctx, a.tenant_id.as_deref()))
            .collect()
    };

    let nodes: Vec<TopoNode> = agents
        .iter()
        .map(|e| TopoNode {
            id: e.id.to_string(),
            name: e.name.clone(),
            state: format!("{:?}", e.state),
            model: e.manifest.model.modality.clone(),
        })
        .collect();

    let mut edges: Vec<TopoEdge> = Vec::new();

    // Parent-child edges from registry
    for agent in &agents {
        for child_id in &agent.children {
            edges.push(TopoEdge {
                from: agent.id.to_string(),
                to: child_id.to_string(),
                kind: EdgeKind::ParentChild,
            });
        }
    }

    // Peer message edges from event bus history
    let events = state.kernel.event_bus.history(500).await;
    let mut peer_pairs = std::collections::HashSet::new();
    for event in &events {
        if let opencarrier_types::event::EventPayload::Message(_) = &event.payload {
            if let opencarrier_types::event::EventTarget::Agent(target_id) = &event.target {
                let from = event.source.to_string();
                let to = target_id.to_string();
                // Deduplicate: only one edge per pair, skip self-loops
                if from != to {
                    let key = if from < to {
                        (from.clone(), to.clone())
                    } else {
                        (to.clone(), from.clone())
                    };
                    if peer_pairs.insert(key) {
                        edges.push(TopoEdge {
                            from,
                            to,
                            kind: EdgeKind::Peer,
                        });
                    }
                }
            }
        }
    }

    Json(serde_json::to_value(Topology { nodes, edges }).unwrap_or_default())
}

/// Filter a kernel event into a CommsEvent, if it represents inter-agent communication.
fn filter_to_comms_event(
    event: &opencarrier_types::event::Event,
    agents: &[opencarrier_types::agent::AgentEntry],
) -> Option<opencarrier_types::comms::CommsEvent> {
    use opencarrier_types::comms::{CommsEvent, CommsEventKind};
    use opencarrier_types::event::{EventPayload, EventTarget, LifecycleEvent};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    match &event.payload {
        EventPayload::Message(msg) => {
            let target_id = match &event.target {
                EventTarget::Agent(id) => id.to_string(),
                _ => String::new(),
            };
            Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentMessage,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: target_id.clone(),
                target_name: resolve_name(&target_id),
                detail: opencarrier_types::truncate_str(&msg.content, 200).to_string(),
            })
        }
        EventPayload::Lifecycle(lifecycle) => match lifecycle {
            LifecycleEvent::Spawned { agent_id, name } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentSpawned,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: name.clone(),
                detail: format!("Agent '{}' spawned", name),
            }),
            LifecycleEvent::Terminated { agent_id, reason } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentTerminated,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: resolve_name(&agent_id.to_string()),
                detail: format!("Terminated: {}", reason),
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Convert an audit entry into a CommsEvent if it represents inter-agent activity.
fn audit_to_comms_event(
    entry: &opencarrier_runtime::audit::AuditEntry,
    agents: &[opencarrier_types::agent::AgentEntry],
) -> Option<opencarrier_types::comms::CommsEvent> {
    use opencarrier_types::comms::{CommsEvent, CommsEventKind};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if id.is_empty() || id == "system" {
                    "system".to_string()
                } else {
                    opencarrier_types::truncate_str(id, 12).to_string()
                }
            })
    };

    let action_str = format!("{:?}", entry.action);
    let (kind, detail, target_label) = match action_str.as_str() {
        "AgentMessage" => {
            // Format detail: "tokens_in=X, tokens_out=Y" → readable summary
            let detail = if entry.detail.starts_with("tokens_in=") {
                let parts: Vec<&str> = entry.detail.split(", ").collect();
                let in_tok = parts
                    .first()
                    .and_then(|p| p.strip_prefix("tokens_in="))
                    .unwrap_or("?");
                let out_tok = parts
                    .get(1)
                    .and_then(|p| p.strip_prefix("tokens_out="))
                    .unwrap_or("?");
                if entry.outcome == "ok" {
                    format!("{} in / {} out tokens", in_tok, out_tok)
                } else {
                    format!(
                        "{} in / {} out — {}",
                        in_tok,
                        out_tok,
                        opencarrier_types::truncate_str(&entry.outcome, 80)
                    )
                }
            } else if entry.outcome != "ok" {
                format!(
                    "{} — {}",
                    opencarrier_types::truncate_str(&entry.detail, 80),
                    opencarrier_types::truncate_str(&entry.outcome, 80)
                )
            } else {
                opencarrier_types::truncate_str(&entry.detail, 200).to_string()
            };
            (CommsEventKind::AgentMessage, detail, "user")
        }
        "AgentSpawn" => (
            CommsEventKind::AgentSpawned,
            format!(
                "Agent spawned: {}",
                opencarrier_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        "AgentKill" => (
            CommsEventKind::AgentTerminated,
            format!(
                "Agent killed: {}",
                opencarrier_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        _ => return None,
    };

    Some(CommsEvent {
        id: format!("audit-{}", entry.seq),
        timestamp: entry.timestamp.clone(),
        kind,
        source_id: entry.agent_id.clone(),
        source_name: resolve_name(&entry.agent_id),
        target_id: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        target_name: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        detail,
    })
}

/// GET /api/comms/events — Return recent inter-agent communication events.
///
/// Sources from both the event bus (for lifecycle events with full context)
/// and the audit log (for message/spawn/kill events that are always captured).
pub async fn comms_events(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);

    let all_agents = state.kernel.registry.list();
    let agents: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|a| can_access(&ctx, a.tenant_id.as_deref()))
            .collect()
    };

    // Primary source: event bus (has full source/target context)
    let bus_events = state.kernel.event_bus.history(500).await;
    let mut comms_events: Vec<opencarrier_types::comms::CommsEvent> = bus_events
        .iter()
        .filter_map(|e| filter_to_comms_event(e, &agents))
        .collect();

    // Secondary source: audit log (always populated, wider coverage)
    let audit_entries = state.kernel.audit_log.recent(500);
    let seen_ids: std::collections::HashSet<String> =
        comms_events.iter().map(|e| e.id.clone()).collect();

    for entry in audit_entries.iter().rev() {
        if let Some(ev) = audit_to_comms_event(entry, &agents) {
            if !seen_ids.contains(&ev.id) {
                comms_events.push(ev);
            }
        }
    }

    // Sort by timestamp descending (newest first)
    comms_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    comms_events.truncate(limit);

    Json(comms_events)
}

/// GET /api/comms/events/stream — SSE stream of inter-agent communication events.
///
/// Polls the audit log every 500ms for new inter-agent events.
pub async fn comms_events_stream(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    use axum::response::sse::{Event, KeepAlive, Sse};

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = {
            let entries = state.kernel.audit_log.recent(1);
            entries.last().map(|e| e.seq).unwrap_or(0)
        };

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let agents = state.kernel.registry.list();
            let entries = state.kernel.audit_log.recent(50);

            for entry in &entries {
                if entry.seq <= last_seq {
                    continue;
                }
                if let Some(comms_event) = audit_to_comms_event(entry, &agents) {
                    let data = serde_json::to_string(&comms_event).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        return; // Client disconnected
                    }
                }
            }

            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// POST /api/comms/send — Send a message from one agent to another.
pub async fn comms_send(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<opencarrier_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);

    // Validate from agent exists and tenant can access it
    let from_id = match parse_agent_id(&req.from_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Some(from_entry) = state.kernel.registry.get(from_id) {
        if !can_access(&ctx, from_entry.tenant_id.as_deref()) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied to source agent"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Source agent not found"})));
    }

    // Validate to agent exists and tenant can access it
    let to_id = match parse_agent_id(&req.to_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Some(to_entry) = state.kernel.registry.get(to_id) {
        if !can_access(&ctx, to_entry.tenant_id.as_deref()) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied to target agent"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Target agent not found"})));
    }

    // SECURITY: Limit message size
    if req.message.len() > 64 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    match state.kernel.send_message(to_id, &req.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "response": result.response,
                "input_tokens": result.total_usage.input_tokens,
                "output_tokens": result.total_usage.output_tokens,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
        ),
    }
}

/// POST /api/comms/task — Post a task to the agent task queue.
pub async fn comms_task(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<opencarrier_types::comms::CommsTaskRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }
    if req.title.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Title is required"})),
        );
    }

    match state
        .kernel
        .memory
        .task_post(
            &req.title,
            &req.description,
            req.assigned_to.as_deref(),
            Some("ui-user"),
        )
        .await
    {
        Ok(task_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "ok": true,
                "task_id": task_id,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to post task: {e}")})),
        ),
    }
}

// ── Dashboard Authentication (username/password sessions) ──

/// POST /api/auth/login — Authenticate with username/password, returns session token.
///
/// First checks the tenants table (multi-tenant login), then falls back to
/// config.toml credentials (legacy admin login).
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::Response;

    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Auth not enabled"}).to_string(),
            ))
            .unwrap();
    }

    let username = req.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = req.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    // Step 1: Try tenants table (multi-tenant login)
    let tenant_store = state.kernel.memory.tenant();
    if let Ok(Some(tenant)) = tenant_store.get_tenant_by_name(username) {
        if !tenant.enabled {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::AuthAttempt,
                "dashboard login failed (tenant disabled)",
                format!("username: {username}"),
            );
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "Invalid credentials"}).to_string(),
                ))
                .unwrap();
        }
        if crate::session_auth::verify_password(password, &tenant.password_hash) {
            let token = crate::session_auth::create_session_token(
                Some(&tenant.id),
                tenant.role.as_str(),
                &tenant.name,
                &secret,
                auth_cfg.session_ttl_hours,
            );
            let ttl_secs = auth_cfg.session_ttl_hours * 3600;
            let cookie = format!(
                "opencarrier_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}"
            );

            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::AuthAttempt,
                "dashboard login success (tenant)",
                format!("username: {username}, role: {}", tenant.role.as_str()),
            );

            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("set-cookie", &cookie)
                .body(Body::from(
                    serde_json::json!({
                        "status": "ok",
                        "token": token,
                        "username": username,
                        "role": tenant.role.as_str(),
                        "tenant_id": tenant.id,
                    })
                    .to_string(),
                ))
                .unwrap();
        }
    }

    // Step 2: Fallback to config.toml credentials (legacy admin login)
    // Constant-time username comparison to prevent timing attacks
    let username_ok = {
        use subtle::ConstantTimeEq;
        let stored = auth_cfg.username.as_bytes();
        let provided = username.as_bytes();
        if stored.len() != provided.len() {
            false
        } else {
            bool::from(stored.ct_eq(provided))
        }
    };

    if !username_ok || !crate::session_auth::verify_password(password, &auth_cfg.password_hash) {
        // Audit log the failed attempt
        state.kernel.audit_log.record(
            "system",
            opencarrier_runtime::audit::AuditAction::AuthAttempt,
            "dashboard login failed",
            format!("username: {username}"),
        );
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Invalid credentials"}).to_string(),
            ))
            .unwrap();
    }

    // Legacy admin login — issue new-format token with admin role
    let token = crate::session_auth::create_session_token(
        None,
        "admin",
        username,
        &secret,
        auth_cfg.session_ttl_hours,
    );
    let ttl_secs = auth_cfg.session_ttl_hours * 3600;
    let cookie = format!(
        "opencarrier_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}"
    );

    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::AuthAttempt,
        "dashboard login success (legacy admin)",
        format!("username: {username}"),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", &cookie)
        .body(Body::from(
            serde_json::json!({
                "status": "ok",
                "token": token,
                "username": username,
                "role": "admin",
            })
            .to_string(),
        ))
        .unwrap()
}

/// POST /api/auth/logout — Clear the session cookie.
pub async fn auth_logout() -> impl IntoResponse {
    let cookie = "opencarrier_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [("content-type", "application/json"), ("set-cookie", cookie)],
        serde_json::json!({"status": "ok"}).to_string(),
    )
}

/// GET /api/auth/check — Check current authentication state.
pub async fn auth_check(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        return Json(serde_json::json!({
            "authenticated": true,
            "mode": "none",
        }));
    }

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    // Check session cookie
    let session_user = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("opencarrier_session=")
                    .map(|v| v.to_string())
            })
        })
        .and_then(|token| crate::session_auth::verify_session_token(&token, &secret));

    if let Some(info) = session_user {
        Json(serde_json::json!({
            "authenticated": true,
            "mode": "session",
            "username": info.username,
            "role": info.role,
            "tenant_id": info.tenant_id,
        }))
    } else {
        Json(serde_json::json!({
            "authenticated": false,
            "mode": "session",
        }))
    }
}

// ========== Tenant Management endpoints ==========

/// Helper: extract TenantContext from request extensions, defaulting to admin.
fn get_tenant_ctx(extensions: &axum::http::Extensions) -> opencarrier_types::tenant::TenantContext {
    extensions
        .get::<opencarrier_types::tenant::TenantContext>()
        .cloned()
        .unwrap_or_else(opencarrier_types::tenant::TenantContext::deny_all)
}

/// Helper: check if the requester can access a resource owned by `resource_tenant_id`.
/// Admin can access everything. Tenants can only access their own resources.
fn can_access(ctx: &opencarrier_types::tenant::TenantContext, resource_tenant_id: Option<&str>) -> bool {
    if ctx.is_admin() {
        return true;
    }
    match (&ctx.tenant_id, resource_tenant_id) {
        (Some(tid), Some(rid)) => tid == rid,
        (Some(_), None) => false, // tenant can't access global resources
        (None, _) => false,        // deny — missing tenant context is not admin
    }
}

/// GET /api/tenants — List all tenants (admin only).
pub async fn list_tenants(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }

    let tenant_store = state.kernel.memory.tenant();
    match tenant_store.list_tenants() {
        Ok(tenants) => {
            // Don't expose password hashes in the API response
            let safe: Vec<serde_json::Value> = tenants
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "name": t.name,
                        "role": t.role.as_str(),
                        "enabled": t.enabled,
                        "created_at": t.created_at,
                        "updated_at": t.updated_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(safe)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to list tenants: {e}")})),
        ),
    }
}

/// POST /api/tenants — Create a new tenant (admin only).
pub async fn create_tenant(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<opencarrier_types::tenant::CreateTenantRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }

    if req.name.trim().is_empty() || req.password.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Name and password are required"})),
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let password_hash = crate::session_auth::hash_password(&req.password);
    let entry = opencarrier_types::tenant::TenantEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name.trim().to_string(),
        password_hash,
        role: opencarrier_types::tenant::TenantRole::Tenant,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };

    let tenant_id = entry.id.clone();
    let tenant_name = entry.name.clone();
    let tenant_store = state.kernel.memory.tenant();
    match tenant_store.create_tenant(&entry) {
        Ok(()) => {
            // Try to auto-start clone-creator and clone-trainer for this tenant
            for clone_name in &["clone-creator", "clone-trainer"] {
                if let Some(existing) = state.kernel.registry.find_by_name(clone_name) {
                    // Already running globally — skip
                    let _ = existing;
                    continue;
                }
                // Check if the clone is installed on disk
                let clone_toml = state
                    .kernel
                    .config
                    .effective_workspaces_dir()
                    .join(clone_name)
                    .join("agent.toml");
                if clone_toml.exists() {
                    match std::fs::read_to_string(&clone_toml) {
                        Ok(toml_str) => {
                            match toml::from_str::<AgentManifest>(&toml_str) {
                                Ok(manifest) => {
                                    match state.kernel.spawn_agent(manifest) {
                                        Ok(agent_id) => {
                                            state.kernel.registry.set_tenant_id(
                                                agent_id,
                                                Some(tenant_id.clone()),
                                            );
                                            tracing::info!(
                                                "Auto-started {} for tenant {}",
                                                clone_name,
                                                tenant_name
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to auto-start {} for tenant {}: {}",
                                                clone_name,
                                                tenant_name,
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Invalid {} manifest: {}", clone_name, e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Cannot read {} manifest: {}", clone_name, e);
                        }
                    }
                }
            }

            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "status": "ok",
                    "id": tenant_id,
                    "name": tenant_name,
                    "role": "tenant",
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create tenant: {e}")})),
        ),
    }
}

/// GET /api/tenants/{id} — Get a single tenant (admin only).
pub async fn get_tenant(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }

    let tenant_store = state.kernel.memory.tenant();
    match tenant_store.get_tenant(&id) {
        Ok(Some(t)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": t.id,
                "name": t.name,
                "role": t.role.as_str(),
                "enabled": t.enabled,
                "created_at": t.created_at,
                "updated_at": t.updated_at,
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Tenant not found"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to get tenant: {e}")})),
        ),
    }
}

/// PUT /api/tenants/{id} — Update a tenant (admin only).
pub async fn update_tenant(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(req): Json<opencarrier_types::tenant::UpdateTenantRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }

    let tenant_store = state.kernel.memory.tenant();
    let mut entry = match tenant_store.get_tenant(&id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Tenant not found"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get tenant: {e}")})),
            );
        }
    };

    if let Some(name) = req.name {
        entry.name = name;
    }
    if let Some(password) = req.password {
        entry.password_hash = crate::session_auth::hash_password(&password);
    }
    if let Some(enabled) = req.enabled {
        entry.enabled = enabled;
    }
    entry.updated_at = chrono::Utc::now().to_rfc3339();

    match tenant_store.update_tenant(&entry) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "id": entry.id,
                "name": entry.name,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update tenant: {e}")})),
        ),
    }
}

/// DELETE /api/tenants/{id} — Delete a tenant (admin only).
pub async fn delete_tenant(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }

    let tenant_store = state.kernel.memory.tenant();
    match tenant_store.delete_tenant(&id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to delete tenant: {e}")})),
        ),
    }
}

// ========== Clone (.agx) endpoints ==========

/// POST /api/clones/install — Install a .agx clone from uploaded bytes.
pub async fn install_clone(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<InstallCloneRequest>,
) -> impl IntoResponse {
    use opencarrier_clone::{load_agx, install_clone_to_workspace, convert_to_manifest};
    let ctx = get_tenant_ctx(&extensions);

    // Decode base64 data
    let raw_data = match req.decode_data() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e})),
            );
        }
    };

    // SECURITY: Reject oversized clone payloads (max 50MB decoded)
    const MAX_CLONE_PAYLOAD: usize = 50 * 1024 * 1024;
    if raw_data.len() > MAX_CLONE_PAYLOAD {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("Clone payload too large: {} bytes (max 50MB)", raw_data.len())
            })),
        );
    }

    // Write uploaded bytes to temp file
    let tmp_dir = std::env::temp_dir().join(format!("opencarrier-clone-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create temp dir: {e}")})),
        );
    }
    let tmp_path = tmp_dir.join("clone.agx");
    if let Err(e) = std::fs::write(&tmp_path, &raw_data) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write temp file: {e}")})),
        );
    }

    // Load and parse .agx
    let clone_data = match load_agx(&tmp_path) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Failed to parse .agx: {e}")})),
            );
        }
    };

    // Clean up temp file
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Check for name collision
    if state.kernel.registry.find_by_name(&clone_data.name).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Agent '{}' already exists", clone_data.name)})),
        );
    }

    // Create workspace directory (tenant-scoped)
    let workspace_dir = state.kernel.config.tenant_workspaces_dir(ctx.tenant_id.as_deref()).join(&clone_data.name);
    if workspace_dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Workspace for '{}' already exists", clone_data.name)})),
        );
    }

    // Install clone files to workspace
    if let Err(e) = install_clone_to_workspace(&clone_data, &workspace_dir) {
        let _ = std::fs::remove_dir_all(&workspace_dir);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to install clone: {e}")})),
        );
    }

    // Convert to AgentManifest
    let mut manifest = convert_to_manifest(&clone_data);
    manifest.workspace = Some(workspace_dir.clone());

    // Spawn agent (tenant-scoped)
    let name = manifest.name.clone();
    let warnings = clone_data.security_warnings.clone();
    let tid = ctx.tenant_id.as_deref();

    match state.kernel.spawn_agent_with_parent(manifest, None, None, tid) {
        Ok(id) => {
            tracing::info!("Clone '{}' installed and spawned: {}", name, id);
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "agent_id": id.to_string(),
                    "name": name,
                    "warnings": warnings,
                })),
            )
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&workspace_dir);
            tracing::warn!("Clone spawn failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to spawn clone agent"})),
            )
        }
    }
}

/// GET /api/clones — List installed clones (agents with clone_source).
pub async fn list_clones(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agents = if ctx.is_admin() {
        state.kernel.registry.list()
    } else {
        state.kernel.registry.list_by_tenant(ctx.tenant_id.as_deref().unwrap_or(""))
    };
    let clones: Vec<serde_json::Value> = agents
        .into_iter()
        .filter(|e| e.manifest.clone_source.is_some())
        .map(|e| {
            let cs = e.manifest.clone_source.as_ref().unwrap();
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "template_name": cs.template_name,
                "template_author": cs.template_author,
                "installed_at": cs.installed_at,
                "knowledge_files": e.manifest.knowledge_files,
                "skills": e.manifest.skills,
            })
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!(clones)))
}

/// POST /api/clones/{name}/start — Start a stopped clone.
pub async fn start_clone(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"}))),
    };
    if !can_access(&ctx, entry.tenant_id.as_deref()) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
    }

    if entry.manifest.clone_source.is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Not a clone agent"})));
    }

    match state.kernel.registry.set_state(entry.id, opencarrier_types::agent::AgentState::Running) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "running"}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))),
    }
}

/// POST /api/clones/{name}/stop — Stop a running clone.
pub async fn stop_clone(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"}))),
    };
    if !can_access(&ctx, entry.tenant_id.as_deref()) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
    }

    if entry.manifest.clone_source.is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Not a clone agent"})));
    }

    match state.kernel.registry.set_state(entry.id, opencarrier_types::agent::AgentState::Suspended) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "suspended"}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))),
    }
}

/// DELETE /api/clones/{name} — Uninstall a clone.
pub async fn uninstall_clone(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"}))),
    };
    if !can_access(&ctx, entry.tenant_id.as_deref()) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
    }

    if entry.manifest.clone_source.is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Not a clone agent"})));
    }

    let agent_id = entry.id;
    let workspace = entry.manifest.workspace.clone();

    // Remove from registry and kill
    match state.kernel.kill_agent(agent_id) {
        Ok(()) => {
            // Remove workspace directory
            if let Some(ws) = workspace {
                let _ = std::fs::remove_dir_all(&ws);
            }
            tracing::info!("Clone '{}' uninstalled", name);
            (StatusCode::OK, Json(serde_json::json!({"status": "uninstalled"})))
        }
        Err(e) => {
            tracing::warn!("Failed to kill clone agent: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")})))
        }
    }
}

/// Request body for clone installation.
#[derive(serde::Deserialize)]
pub struct InstallCloneRequest {
    /// Base64-encoded .agx file bytes.
    pub data: String,
}

impl InstallCloneRequest {
    /// Decode base64 data to raw bytes.
    pub fn decode_data(&self) -> Result<Vec<u8>, String> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|e| format!("Invalid base64 data: {e}"))
    }
}

/// Run knowledge compile for a clone agent.
///
/// POST /api/clones/{name}/compile
///
/// Triggers metadata generation, overlap merging, stale/expiry cleanup,
/// and compression on the clone's knowledge directory.
pub async fn clone_compile(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    // Resolve an LLM driver for compile operations
    let driver = match state.kernel.resolve_driver(&entry.manifest) {
        Ok(d) => d,
        Err(_) => {
            return (
                StatusCode::FAILED_DEPENDENCY,
                Json(serde_json::json!({"error": "No LLM driver available for compile"})),
            )
        }
    };

    let config = opencarrier_lifecycle::evolution_config::read_evolution_config(&workspace);

    // Run compile in a blocking thread with an async LLM callback
    let result = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let llm_call = |sys: &str, user: &str, max_tokens: u32| -> anyhow::Result<String> {
            let request = opencarrier_runtime::llm_driver::CompletionRequest {
                model: String::new(),
                messages: vec![opencarrier_types::message::Message {
                    role: opencarrier_types::message::Role::User,
                    content: opencarrier_types::message::MessageContent::Text(user.to_string()),
                }],
                tools: vec![],
                max_tokens,
                temperature: 0.3,
                system: Some(sys.to_string()),
                thinking: None,
            };
            rt.block_on(async { driver.complete(request).await })
                .map(|r: opencarrier_runtime::llm_driver::CompletionResponse| r.text())
                .map_err(|e| anyhow::anyhow!("{e}"))
        };

        opencarrier_lifecycle::compile::run_compile(&workspace, &config, &llm_call)
    })
    .await;

    match result {
        Ok(result) => {
            tracing::info!(
                clone = %name,
                metadata = result.metadata_generated,
                merged = result.files_merged,
                stale = result.stale_marked,
                deleted = result.expired_deleted,
                compressed = result.files_compressed,
                skipped = result.skipped_unchanged,
                errors = result.errors.len(),
                "Compile complete"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "compiled",
                    "metadata_generated": result.metadata_generated,
                    "files_merged": result.files_merged,
                    "stale_marked": result.stale_marked,
                    "expired_deleted": result.expired_deleted,
                    "files_compressed": result.files_compressed,
                    "skipped_unchanged": result.skipped_unchanged,
                    "errors": result.errors,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Compile task panicked: {e}")})),
        ),
    }
}

/// Run health check for a clone agent's knowledge directory.
///
/// GET /api/clones/{name}/health
///
/// Returns a health report with warnings and errors. Optionally auto-fixes
/// issues when `?fix=true` is passed.
pub async fn clone_health(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let do_fix = params
        .get("fix")
        .map(|v| v == "true")
        .unwrap_or(false);

    let report = opencarrier_lifecycle::health::check_health(&workspace);

    if do_fix {
        let fixes = opencarrier_lifecycle::health::auto_fix(&workspace, &report);
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "report": report,
                "fixes_applied": fixes,
            })),
        );
    }

    (StatusCode::OK, Json(serde_json::json!({"report": report})))
}

/// Push collected feedback to Hub.
///
/// POST /api/clones/{name}/feedback/push
///
/// Collects all feedback entries from `feedback/*.json` and pushes them
/// to the configured Hub.
pub async fn clone_feedback_push(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let entries = match opencarrier_lifecycle::feedback::collect_feedback(&workspace) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to collect feedback: {e}")})),
            )
        }
    };

    if entries.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "no_feedback", "count": 0})),
        );
    }

    let hub_url = state.kernel.config.hub.url.clone();
    let hub_api_key =
        opencarrier_clone::hub::read_api_key(&state.kernel.config.hub.api_key_env)
            .unwrap_or_default();

    match opencarrier_lifecycle::feedback::push_feedback_to_hub(&hub_url, &hub_api_key, &entries)
        .await
    {
        Ok(results) => {
            let pushed = results.iter().filter(|r| r.starts_with("ok:")).count();
            let failed = results.len() - pushed;
            tracing::info!(
                clone = %name,
                pushed = pushed,
                failed = failed,
                "Feedback pushed to Hub"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "pushed",
                    "total": entries.len(),
                    "pushed": pushed,
                    "failed": failed,
                    "results": results,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Push failed: {e}")})),
        ),
    }
}

/// Evaluate clone quality — deterministic metrics + optional LLM assessment.
///
/// GET /api/clones/{name}/evaluate?mode=deterministic|full
pub async fn clone_evaluate(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let metrics = opencarrier_lifecycle::evaluate::compute_deterministic_metrics(&workspace);

    let mode = params.get("mode").map(|s| s.as_str()).unwrap_or("deterministic");

    if mode == "full" {
        // Full mode: generate test questions from knowledge, ask clone, judge answers.
        let knowledge_content =
            opencarrier_lifecycle::evaluate::read_knowledge_for_eval(&workspace);
        let mut questions: Vec<opencarrier_lifecycle::evaluate::EvalQuestion> = Vec::new();
        let mut avg_llm_score: Option<f32> = None;

        if !knowledge_content.is_empty() {
            if let Ok(driver) = state.kernel.resolve_driver(&entry.manifest) {
                let (sys_prompt, user_prompt) =
                    opencarrier_lifecycle::evaluate::build_test_questions_prompt(&knowledge_content);

                // Generate test questions
                let response_text = match driver
                    .complete(opencarrier_runtime::llm_driver::CompletionRequest {
                        model: String::new(), // driver uses its default
                        messages: vec![opencarrier_types::message::Message {
                            role: opencarrier_types::message::Role::User,
                            content: opencarrier_types::message::MessageContent::Text(user_prompt),
                        }],
                        tools: vec![],
                        max_tokens: 1024,
                        temperature: 0.7,
                        system: Some(sys_prompt),
                        thinking: None,
                    })
                    .await
                {
                    Ok(resp) => resp.text(),
                    Err(_) => String::new(),
                };

                let test_qs = opencarrier_lifecycle::evaluate::parse_test_questions(&response_text);

                if !test_qs.is_empty() {
                    let mut scores: Vec<f32> = Vec::new();
                    for q in &test_qs {
                        // Ask the clone
                        let answer_text = match driver
                            .complete(opencarrier_runtime::llm_driver::CompletionRequest {
                                model: String::new(), // driver uses its default
                                messages: vec![opencarrier_types::message::Message {
                                    role: opencarrier_types::message::Role::User,
                                    content: opencarrier_types::message::MessageContent::Text(
                                        q.clone(),
                                    ),
                                }],
                                tools: vec![],
                                max_tokens: 1024,
                                temperature: 0.3,
                                system: Some("Answer the following question concisely.".to_string()),
                                thinking: None,
                            })
                            .await
                        {
                            Ok(resp) => resp.text(),
                            Err(_) => continue,
                        };

                        // Judge the answer
                        let (j_sys, j_user) =
                            opencarrier_lifecycle::evaluate::build_judge_prompt(q, &answer_text);
                        let judge_text = match driver
                            .complete(opencarrier_runtime::llm_driver::CompletionRequest {
                                model: String::new(), // driver uses its default
                                messages: vec![opencarrier_types::message::Message {
                                    role: opencarrier_types::message::Role::User,
                                    content: opencarrier_types::message::MessageContent::Text(
                                        j_user,
                                    ),
                                }],
                                tools: vec![],
                                max_tokens: 256,
                                temperature: 0.0,
                                system: Some(j_sys),
                                thinking: None,
                            })
                            .await
                        {
                            Ok(resp) => resp.text(),
                            Err(_) => continue,
                        };

                        let (score, feedback) =
                            opencarrier_lifecycle::evaluate::parse_judge_response(&judge_text);
                        scores.push(score);
                        questions.push(opencarrier_lifecycle::evaluate::EvalQuestion {
                            question: q.clone(),
                            score,
                            feedback,
                        });
                    }
                    if !scores.is_empty() {
                        avg_llm_score = Some(scores.iter().sum::<f32>() / scores.len() as f32);
                    }
                }
            }
        }

        let report = opencarrier_lifecycle::evaluate::EvalReport {
            metrics,
            questions,
            avg_llm_score,
        };
        return (StatusCode::OK, Json(serde_json::json!(report)));
    }

    // Deterministic-only mode
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "metrics": metrics,
        })),
    )
}

/// Rollback a knowledge file to its previous version.
///
/// POST /api/clones/{name}/rollback
/// Body: { "filename": "refund-policy.md" }
pub async fn clone_rollback(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let filename = match body["filename"].as_str() {
        Some(f) => f.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'filename' field"})),
            )
        }
    };

    let (_entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match opencarrier_lifecycle::version::rollback_file(&workspace, &filename) {
        Ok(()) => {
            tracing::info!(clone = %name, file = %filename, "Knowledge file rolled back");
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "rolled_back", "filename": filename})),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// Verify (approve) the latest unverified version of a knowledge file.
///
/// POST /api/clones/{name}/verify
/// Body: { "filename": "refund-policy.md" }
pub async fn clone_verify(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let filename = match body["filename"].as_str() {
        Some(f) => f.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'filename' field"})),
            )
        }
    };

    let (_entry, workspace) = match get_clone_workspace_with_tenant(&name, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match opencarrier_lifecycle::version::verify_version(&workspace, &filename) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "verified", "filename": filename})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ========== Hub template marketplace endpoints ==========

/// GET /api/hub/templates — List templates from the connected Hub.
pub async fn list_hub_templates(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let hub_url = state.kernel.config.hub.url.clone();
    // SECURITY: Validate hub URL before fetching
    if let Err(e) = opencarrier_clone::hub::validate_hub_url(&hub_url) {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()})));
    }
    let hub_api_key = match
        opencarrier_clone::hub::read_api_key(&state.kernel.config.hub.api_key_env)
    {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": format!("Hub API key not configured: {e}")
                })),
            );
        }
    };

    let url = format!("{}/api/templates?limit=50", hub_url.trim_end_matches('/'));

    let resp = match reqwest::Client::new()
        .get(&url)
        .bearer_auth(&hub_api_key)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("Hub unreachable: {e}")})),
            );
        }
    };

    if !resp.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": format!("Hub returned {}", resp.status())
            })),
        );
    }

    match resp.json::<serde_json::Value>().await {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("Failed to parse Hub response: {e}")})),
        ),
    }
}

/// POST /api/hub/templates/{name}/install — Download and install a template from Hub.
pub async fn install_hub_template(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let _ = &ctx; // Will be used when clone_install kernel tool accepts tenant_id
    let hub_url = state.kernel.config.hub.url.clone();
    // SECURITY: Validate hub URL before fetching
    if let Err(e) = opencarrier_clone::hub::validate_hub_url(&hub_url) {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()})));
    }
    let hub_api_key = match
        opencarrier_clone::hub::read_api_key(&state.kernel.config.hub.api_key_env)
    {
        Ok(k) => {
            tracing::info!(key_env = %state.kernel.config.hub.api_key_env, key_prefix = &k[..8.min(k.len())], "Hub API key loaded");
            k
        }
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": format!("Hub API key not configured: {e}")
                })),
            );
        }
    };

    let home_dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let device_id = opencarrier_clone::hub::get_or_create_device_id(&home_dir)
        .unwrap_or_else(|_| "unknown".to_string());

    let base = hub_url.trim_end_matches('/');
    let download_url = format!(
        "{}/api/templates/{}/download",
        base,
        urlencoding::encode(&name)
    );

    tracing::info!(template = %name, "Downloading from Hub for install");

    let resp = match reqwest::Client::new()
        .get(&download_url)
        .bearer_auth(&hub_api_key)
        .header("X-Device-ID", &device_id)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("Hub unreachable: {e}")})),
            );
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("Hub download failed: {} — {}", status, body)})),
        );
    }

    let agx_bytes = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("Failed to read download: {e}")})),
            );
        }
    };

    match state.kernel.clone_install(&name, &agx_bytes).await {
        Ok((agent_id, agent_name)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "agent_id": agent_id,
                "name": agent_name,
                "size": agx_bytes.len(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Plugin system
// ---------------------------------------------------------------------------

/// GET /api/plugins — list loaded plugin tool status.
pub async fn plugins_list(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Get plugin status from PluginManager if available
    let plugin_statuses = if let Some(ref pm) = state.plugin_manager {
        let pm = pm.lock().await;
        let statuses = pm.status();
        drop(pm);
        statuses
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "version": s.version,
                    "loaded": s.loaded,
                    "channels": s.channels,
                    "tools": s.tools,
                })
            })
            .collect::<Vec<_>>()
    } else {
        // Fallback: read from kernel's tool dispatcher
        let guard = state.kernel.plugin_tool_dispatcher.lock().unwrap();
        if let Some(ref dispatcher) = *guard {
            dispatcher
                .definitions()
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect()
        } else {
            vec![]
        }
    };

    Json(serde_json::json!({
        "plugins": plugin_statuses,
        "count": plugin_statuses.len(),
    }))
}

// ---------------------------------------------------------------------------
// WeChat iLink Bot — QR code login & status
// ---------------------------------------------------------------------------

/// WeChat iLink API base URL.
const WEIXIN_ILINK_BASE: &str = "https://ilinkai.weixin.qq.com";
/// iLink bot_type for personal account.
const WEIXIN_BOT_TYPE: u32 = 3;

/// Validate tenant name: only alphanumeric, hyphen, underscore. Prevents path traversal.
fn weixin_sanitize_tenant(name: &str) -> Option<&str> {
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    if name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        Some(name)
    } else {
        None
    }
}

/// Build a shared reqwest client for iLink API calls (no-redirect, no proxy tricks).
fn weixin_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

/// Validate that a baseurl from iLink response is safe (must match known iLink domain).
fn weixin_validate_baseurl(url: &str) -> bool {
    url.starts_with("https://ilinkai.weixin.qq.com")
        || url.starts_with("https://ilinkai.weixin.qq.com/")
}

/// Atomic file write: write to `<path>.tmp` then rename over target.
fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp_path = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)
}

/// GET `/api/weixin/qrcode` — fetch a fresh QR code for WeChat scanning.
///
/// Query params: `?tenant=<name>` (optional, defaults to "default")
pub async fn weixin_qrcode(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let raw_tenant = params.get("tenant").map(|s| s.as_str()).unwrap_or("default");
    let tenant = match weixin_sanitize_tenant(raw_tenant) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
            );
        }
    };

    let url = format!(
        "{WEIXIN_ILINK_BASE}/ilink/bot/get_bot_qrcode?bot_type={WEIXIN_BOT_TYPE}"
    );

    let http = weixin_http_client();
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(tenant, "get_bot_qrcode request failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("iLink request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(tenant, %status, "get_bot_qrcode returned {status}: {body}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("iLink HTTP {status}") })),
        );
    }

    match resp.json::<serde_json::Value>().await {
        Ok(data) => (StatusCode::OK, Json(serde_json::json!({
            "tenant": tenant,
            "data": data,
        }))),
        Err(e) => {
            tracing::error!(tenant, "get_bot_qrcode parse error: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            )
        }
    }
}

/// GET `/api/weixin/qrcode-status` — poll QR code scan status.
///
/// Query params: `?tenant=<name>&qrcode=<code>`
///
/// When status becomes "confirmed", saves the bot_token and registers the tenant.
pub async fn weixin_qrcode_status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let raw_tenant = params.get("tenant").map(|s| s.as_str()).unwrap_or("default");
    let tenant = match weixin_sanitize_tenant(raw_tenant) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid tenant name" })),
            );
        }
    };
    let qrcode = match params.get("qrcode") {
        Some(q) => q.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing qrcode parameter" })),
            );
        }
    };

    let url = format!(
        "{WEIXIN_ILINK_BASE}/ilink/bot/get_qrcode_status?qrcode={}",
        urlencoding::encode(&qrcode)
    );

    let http = weixin_http_client();
    let resp = match http
        .get(&url)
        .timeout(std::time::Duration::from_secs(40))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status request failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("iLink request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(tenant, %status, "get_qrcode_status returned {status}: {body}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("iLink HTTP {status}") })),
        );
    }

    // iLink may return application/octet-stream
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status read body error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Read error: {e}") })),
            );
        }
    };

    let data: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(tenant, "get_qrcode_status parse error: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            );
        }
    };

    // Check if scan is confirmed — if so, extract bot_token and register tenant
    let scan_status = data
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if scan_status == "confirmed" {
        let bot_token = data
            .get("bot_token")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let raw_baseurl = data
            .get("baseurl")
            .and_then(|v| v.as_str())
            .unwrap_or(WEIXIN_ILINK_BASE);
        let ilink_bot_id = data
            .get("ilink_bot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ilink_user_id = data.get("ilink_user_id").and_then(|v| v.as_str());

        // Validate baseurl to prevent stored SSRF
        let baseurl = if weixin_validate_baseurl(raw_baseurl) {
            raw_baseurl
        } else {
            tracing::warn!(tenant, raw_baseurl, "iLink returned unexpected baseurl, falling back to default");
            WEIXIN_ILINK_BASE
        };

        if !bot_token.is_empty() && !ilink_bot_id.is_empty() {
            // Save token to disk so plugin can pick it up on restart
            let token_dir = state.kernel.config.home_dir.join("weixin-tokens");
            if let Err(e) = std::fs::create_dir_all(&token_dir) {
                tracing::error!(tenant, "Failed to create weixin token dir: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to create token directory: {e}") })),
                );
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let token_file = serde_json::json!({
                "name": tenant,
                "bot_token": bot_token,
                "baseurl": baseurl,
                "ilink_bot_id": ilink_bot_id,
                "user_id": ilink_user_id,
                "expires_at": now + 86400, // 24h
                "bind_agent": null,
            });
            let path = token_dir.join(format!("{tenant}.json"));
            match serde_json::to_string_pretty(&token_file) {
                Ok(json) => {
                    if let Err(e) = atomic_write(&path, &json) {
                        tracing::error!(tenant, "Failed to write weixin token file: {e}");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("Failed to save token: {e}") })),
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(tenant, "Failed to serialize token file: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": "Internal serialization error" })),
                    );
                }
            }

            tracing::info!(
                tenant,
                ilink_bot_id,
                "WeChat iLink QR scan confirmed — token saved"
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tenant": tenant,
            "status": scan_status,
            "data": data,
        })),
    )
}

/// GET `/api/weixin/status` — list all bound WeChat accounts with expiry info.
pub async fn weixin_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let token_dir = state.kernel.config.home_dir.join("weixin-tokens");

    let mut tenants: Vec<serde_json::Value> = Vec::new();

    if token_dir.exists() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(entries) = std::fs::read_dir(&token_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                        let expires_at = tf
                            .get("expires_at")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let expired = now >= expires_at;
                        let remaining = (expires_at - now).max(0);

                        tenants.push(serde_json::json!({
                            "name": tf.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            "ilink_bot_id": tf.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or(""),
                            "user_id": tf.get("user_id").and_then(|v| v.as_str()),
                            "expires_at": expires_at,
                            "remaining_secs": remaining,
                            "expired": expired,
                            "bind_agent": tf.get("bind_agent").and_then(|v| v.as_str()),
                        }));
                    }
                }
            }
        }
    }

    Json(serde_json::json!({
        "tenants": tenants,
        "count": tenants.len(),
    }))
}

// ---------------------------------------------------------------------------
// Channels — unified status + tenant management
// ---------------------------------------------------------------------------

/// GET `/api/channels/status` — aggregate status for all channel plugins.
///
/// Reads WeChat token files, WeCom and Feishu plugin.toml tenants.
pub async fn channels_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let home = &state.kernel.config.home_dir;

    // ── WeChat iLink ──────────────────────────────────────────────────
    let weixin_dir = home.join("weixin-tokens");
    let mut weixin_tenants: Vec<serde_json::Value> = Vec::new();

    if weixin_dir.exists() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(entries) = std::fs::read_dir(&weixin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                        let expires_at = tf.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
                        let expired = now >= expires_at;
                        let remaining = (expires_at - now).max(0);
                        weixin_tenants.push(serde_json::json!({
                            "name": tf.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            "ilink_bot_id": tf.get("ilink_bot_id").and_then(|v| v.as_str()).unwrap_or(""),
                            "expired": expired,
                            "remaining_secs": remaining,
                        }));
                    }
                }
            }
        }
    }

    // ── WeCom ──────────────────────────────────────────────────────────
    let wecom_toml = home.join("plugins").join("opencarrier-plugin-wecom").join("plugin.toml");
    let mut wecom_tenants: Vec<serde_json::Value> = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&wecom_toml) {
        if let Ok(doc) = content.parse::<toml::Value>() {
            if let Some(arr) = doc.get("tenants").and_then(|v| v.as_array()) {
                for tenant in arr {
                    let cfg = tenant.get("config").cloned().unwrap_or(toml::Value::Table(Default::default()));
                    wecom_tenants.push(serde_json::json!({
                        "name": tenant.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "mode": cfg.get("mode").and_then(|v| v.as_str()).unwrap_or("smartbot"),
                        "corp_id": cfg.get("corp_id").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
            }
        }
    }

    // ── Feishu/Lark ────────────────────────────────────────────────────
    let feishu_toml = home.join("plugins").join("opencarrier-plugin-feishu").join("plugin.toml");
    let mut feishu_tenants: Vec<serde_json::Value> = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&feishu_toml) {
        if let Ok(doc) = content.parse::<toml::Value>() {
            if let Some(arr) = doc.get("tenants").and_then(|v| v.as_array()) {
                for tenant in arr {
                    let cfg = tenant.get("config").cloned().unwrap_or(toml::Value::Table(Default::default()));
                    feishu_tenants.push(serde_json::json!({
                        "name": tenant.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "app_id": cfg.get("app_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "brand": cfg.get("brand").and_then(|v| v.as_str()).unwrap_or("feishu"),
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "weixin": { "tenants": weixin_tenants, "count": weixin_tenants.len() },
        "wecom": { "tenants": wecom_tenants, "count": wecom_tenants.len() },
        "feishu": { "tenants": feishu_tenants, "count": feishu_tenants.len() },
    }))
}

/// Sanitize tenant name for plugin.toml entries.
fn channel_sanitize_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return None;
    }
    if trimmed.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Maximum length for config string fields (corp_id, secret, etc.).
const CHANNEL_FIELD_MAX_LEN: usize = 512;

/// Validate a config string field: non-empty after trim, max length, no control chars.
fn channel_validate_field(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    if trimmed.len() > CHANNEL_FIELD_MAX_LEN {
        return Err(format!("{field_name} exceeds max length ({CHANNEL_FIELD_MAX_LEN} chars)"));
    }
    if trimmed.chars().any(|c| c.is_control() && c != ' ') {
        return Err(format!("{field_name} contains invalid characters"));
    }
    Ok(trimmed.to_string())
}

/// Read-modify-write a plugin.toml, appending a new [[tenants]] entry.
/// Uses file locking to prevent concurrent write races.
fn plugin_toml_add_tenant(
    toml_path: &std::path::Path,
    tenant_name: &str,
    config: toml::Value,
) -> Result<(), String> {
    // Use file-based lock to prevent TOCTOU race on concurrent writes.
    // The lock file is adjacent to the target toml file.
    let lock_path = {
        let mut s = toml_path.as_os_str().to_owned();
        s.push(".lock");
        std::path::PathBuf::from(s)
    };

    // Create parent dir if needed
    if let Some(parent) = toml_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Failed to create plugin directory".to_string())?;
    }

    // Acquire exclusive lock
    let lock_file = std::fs::File::create(&lock_path)
        .map_err(|_| "Failed to create lock file".to_string())?;
    lock_file.lock_exclusive()
        .map_err(|_| "Failed to acquire config lock".to_string())?;

    let result = plugin_toml_add_tenant_locked(toml_path, tenant_name, config);

    // Release lock (drop closes the file)
    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);

    result
}

/// Inner implementation (caller must hold the lock).
fn plugin_toml_add_tenant_locked(
    toml_path: &std::path::Path,
    tenant_name: &str,
    config: toml::Value,
) -> Result<(), String> {
    let mut doc = if toml_path.exists() {
        let content = std::fs::read_to_string(toml_path)
            .map_err(|_| "Failed to read plugin config".to_string())?;
        content
            .parse::<toml::Value>()
            .map_err(|_| "Failed to parse plugin config".to_string())?
    } else {
        toml::Value::Table(Default::default())
    };

    let table = doc.as_table_mut().ok_or("Invalid plugin config structure".to_string())?;
    if !table.contains_key("tenants") {
        table.insert("tenants".into(), toml::Value::Array(Vec::new()));
    }
    let tenants = table
        .get_mut("tenants")
        .and_then(|v| v.as_array_mut())
        .ok_or("Invalid tenants section".to_string())?;

    // Check for duplicate name
    if tenants.iter().any(|t| {
        t.get("name").and_then(|v| v.as_str()) == Some(tenant_name)
    }) {
        return Err(format!("Tenant '{tenant_name}' already exists"));
    }

    let mut entry = toml::value::Table::new();
    entry.insert("name".into(), toml::Value::String(tenant_name.to_string()));
    entry.insert("config".into(), config);
    tenants.push(toml::Value::Table(entry));

    let content = toml::to_string_pretty(&doc)
        .map_err(|_| "Failed to serialize plugin config".to_string())?;
    atomic_write(toml_path, &content)
        .map_err(|_| "Failed to write plugin config".to_string())?;

    Ok(())
}

/// POST `/api/channels/wecom/tenants` — add a WeCom tenant to plugin.toml.
///
/// Body: `{ "name": "...", "mode": "smartbot"|"app"|"kf", "corp_id": "...", "bot_id": "...", "secret": "...", "webhook_port": 8454, "encoding_aes_key": "..." }`
pub async fn wecom_add_tenant(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => match channel_sanitize_name(n) {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing 'name' field" })),
            );
        }
    };

    let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("smartbot");
    if !["smartbot", "app", "kf"].contains(&mode) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid mode: must be smartbot, app, or kf" })),
        );
    }

    let corp_id = match channel_validate_field(
        body.get("corp_id").and_then(|v| v.as_str()).unwrap_or(""), "corp_id",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let secret = match channel_validate_field(
        body.get("secret").and_then(|v| v.as_str()).unwrap_or(""), "secret",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let bot_id = body.get("bot_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if bot_id.len() > CHANNEL_FIELD_MAX_LEN || bot_id.chars().any(|c| c.is_control() && c != ' ') {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid bot_id" })));
    }
    let webhook_port = body.get("webhook_port").and_then(|v| v.as_u64()).unwrap_or(8454);
    if !(1..=65535).contains(&webhook_port) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "webhook_port must be between 1 and 65535" })),
        );
    }
    let encoding_aes_key = body.get("encoding_aes_key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if encoding_aes_key.len() > CHANNEL_FIELD_MAX_LEN || encoding_aes_key.chars().any(|c| c.is_control() && c != ' ') {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid encoding_aes_key" })));
    }

    // Build config as toml::Value
    let mut cfg = toml::value::Table::new();
    cfg.insert("mode".into(), toml::Value::String(mode.to_string()));
    cfg.insert("corp_id".into(), toml::Value::String(corp_id.to_string()));
    if !bot_id.is_empty() {
        cfg.insert("bot_id".into(), toml::Value::String(bot_id.to_string()));
    }
    cfg.insert("secret".into(), toml::Value::String(secret.to_string()));
    cfg.insert("webhook_port".into(), toml::Value::Integer(webhook_port as i64));
    if !encoding_aes_key.is_empty() {
        cfg.insert("encoding_aes_key".into(), toml::Value::String(encoding_aes_key.to_string()));
    }

    let toml_path = state.kernel.config.home_dir
        .join("plugins")
        .join("opencarrier-plugin-wecom")
        .join("plugin.toml");

    if let Err(e) = plugin_toml_add_tenant(&toml_path, &name, toml::Value::Table(cfg)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, mode, "WeCom tenant added via dashboard");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "name": name })))
}

/// POST `/api/channels/feishu/tenants` — add a Feishu tenant to plugin.toml.
///
/// Body: `{ "name": "...", "app_id": "...", "app_secret": "...", "brand": "feishu"|"lark" }`
pub async fn feishu_add_tenant(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => match channel_sanitize_name(n) {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Invalid tenant name: use only alphanumeric, hyphen, underscore (max 64 chars)" })),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing 'name' field" })),
            );
        }
    };

    let app_id = match channel_validate_field(
        body.get("app_id").and_then(|v| v.as_str()).unwrap_or(""), "app_id",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let app_secret = match channel_validate_field(
        body.get("app_secret").and_then(|v| v.as_str()).unwrap_or(""), "app_secret",
    ) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
    };
    let brand = body.get("brand").and_then(|v| v.as_str()).unwrap_or("feishu");

    if !["feishu", "lark"].contains(&brand) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid brand: must be feishu or lark" })),
        );
    }

    let mut cfg = toml::value::Table::new();
    cfg.insert("app_id".into(), toml::Value::String(app_id.to_string()));
    cfg.insert("app_secret".into(), toml::Value::String(app_secret.to_string()));
    cfg.insert("brand".into(), toml::Value::String(brand.to_string()));

    let toml_path = state.kernel.config.home_dir
        .join("plugins")
        .join("opencarrier-plugin-feishu")
        .join("plugin.toml");

    if let Err(e) = plugin_toml_add_tenant(&toml_path, &name, toml::Value::Table(cfg)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        );
    }

    tracing::info!(tenant = %name, brand, "Feishu tenant added via dashboard");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "name": name })))
}
