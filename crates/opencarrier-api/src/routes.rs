//! Route handlers for the OpenCarrier API.

use crate::types::*;
use axum::extract::{Path, Query, State};
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

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// POST /api/agents — Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
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
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
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
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
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

    // Check agent exists before processing
    match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(_) => (), Err(r) => return r };

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
                    cost_usd: result.cost_usd,
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

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
                                                filename: format!(
                                                    "image.{}",
                                                    media_type.rsplit('/').next().unwrap_or("png")
                                                ),
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
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Check agent exists
    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

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
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
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

// ── Brain config management ────────────────────────────────────────────────

/// PUT /api/brain/providers/{name} — create or update a Brain provider.
pub async fn set_brain_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let api_key_env = body["api_key_env"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    let result = state.kernel.update_brain(|config| {
        config.providers.insert(
            name.clone(),
            opencarrier_types::brain::ProviderConfig { api_key_env, params: std::collections::HashMap::new() },
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
    Path(name): Path<String>,
) -> impl IntoResponse {
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
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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
    Path(name): Path<String>,
) -> impl IntoResponse {
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
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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
    Path(name): Path<String>,
) -> impl IntoResponse {
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
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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
pub async fn reload_brain(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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

/// POST /api/shutdown — Graceful shutdown.
pub async fn shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
    Json(serde_json::json!({"status": "shutting_down"}))
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
    Path(id): Path<String>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

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

    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp.into_response(),
    };

    match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(_) => (), Err(r) => return r.into_response() };

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
    let agents_dir = opencarrier_kernel::config::opencarrier_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

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
///
/// Note: memory_store tool writes to a shared namespace, so we read from that
/// same namespace regardless of which agent ID is in the URL.
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    let agent_id = opencarrier_kernel::kernel::shared_memory_agent_id();

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
    Path((_id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = opencarrier_kernel::kernel::shared_memory_agent_id();

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
    Path((_id, key)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = opencarrier_kernel::kernel::shared_memory_agent_id();

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
    Path((_id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = opencarrier_kernel::kernel::shared_memory_agent_id();

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
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
    )
}

// ---------------------------------------------------------------------------
// Skills endpoints
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
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
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
    }))
}

/// GET /api/audit/verify — Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })),
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
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
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
pub async fn usage_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
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
pub async fn usage_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_summary(None) {
        Ok(s) => Json(serde_json::json!({
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "total_cost_usd": s.total_cost_usd,
            "call_count": s.call_count,
            "total_tool_calls": s.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "total_cost_usd": 0.0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}

/// GET /api/usage/by-model — Get usage grouped by model.
pub async fn usage_by_model(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_by_model() {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_cost_usd": m.total_cost_usd,
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
pub async fn usage_daily(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let days = state.kernel.memory.usage().query_daily_breakdown(7);
    let today_cost = state.kernel.memory.usage().query_today_cost();
    let first_event = state.kernel.memory.usage().query_first_event_date();

    let days_list = match days {
        Ok(d) => d
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "cost_usd": day.cost_usd,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "today_cost_usd": today_cost.unwrap_or(0.0),
        "first_event_date": first_event.unwrap_or(None),
    }))
}

// ---------------------------------------------------------------------------
// Budget endpoints
// ---------------------------------------------------------------------------

/// GET /api/budget — Current budget status (limits, spend, % used).
pub async fn budget_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = state
        .kernel
        .metering
        .budget_status(&state.kernel.config.budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// PUT /api/budget — Update global budget limits (in-memory only, not persisted to config.toml).
pub async fn update_budget(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // SAFETY: Budget config is updated in-place. Since KernelConfig is behind
    // an Arc and we only have &self, we use ptr mutation (same pattern as OFP).
    let config_ptr = &state.kernel.config as *const opencarrier_types::config::KernelConfig
        as *mut opencarrier_types::config::KernelConfig;

    // Apply updates
    unsafe {
        if let Some(v) = body["max_hourly_usd"].as_f64() {
            (*config_ptr).budget.max_hourly_usd = v;
        }
        if let Some(v) = body["max_daily_usd"].as_f64() {
            (*config_ptr).budget.max_daily_usd = v;
        }
        if let Some(v) = body["max_monthly_usd"].as_f64() {
            (*config_ptr).budget.max_monthly_usd = v;
        }
        if let Some(v) = body["alert_threshold"].as_f64() {
            (*config_ptr).budget.alert_threshold = v.clamp(0.0, 1.0);
        }
        if let Some(v) = body["default_max_llm_tokens_per_hour"].as_u64() {
            (*config_ptr).budget.default_max_llm_tokens_per_hour = v;
        }
    }

    let status = state
        .kernel
        .metering
        .budget_status(&state.kernel.config.budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// GET /api/budget/agents/{id} — Per-agent budget/quota status.
pub async fn agent_budget_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    let quota = &entry.manifest.resources;
    let usage_store = opencarrier_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let hourly = usage_store.query_hourly(agent_id).unwrap_or(0.0);
    let daily = usage_store.query_daily(agent_id).unwrap_or(0.0);
    let monthly = usage_store.query_monthly(agent_id).unwrap_or(0.0);

    // Token usage from scheduler
    let token_usage = state.kernel.scheduler.get_usage(agent_id);
    let tokens_used = token_usage.map(|(t, _)| t).unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "agent_name": entry.name,
            "hourly": {
                "spend": hourly,
                "limit": quota.max_cost_per_hour_usd,
                "pct": if quota.max_cost_per_hour_usd > 0.0 { hourly / quota.max_cost_per_hour_usd } else { 0.0 },
            },
            "daily": {
                "spend": daily,
                "limit": quota.max_cost_per_day_usd,
                "pct": if quota.max_cost_per_day_usd > 0.0 { daily / quota.max_cost_per_day_usd } else { 0.0 },
            },
            "monthly": {
                "spend": monthly,
                "limit": quota.max_cost_per_month_usd,
                "pct": if quota.max_cost_per_month_usd > 0.0 { monthly / quota.max_cost_per_month_usd } else { 0.0 },
            },
            "tokens": {
                "used": tokens_used,
                "limit": quota.max_llm_tokens_per_hour,
                "pct": if quota.max_llm_tokens_per_hour > 0 { tokens_used as f64 / quota.max_llm_tokens_per_hour as f64 } else { 0.0 },
            },
        })),
    )
}

/// GET /api/budget/agents — Per-agent cost ranking (top spenders).
pub async fn agent_budget_ranking(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let usage_store = opencarrier_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .filter_map(|entry| {
            let daily = usage_store.query_daily(entry.id).unwrap_or(0.0);
            if daily > 0.0 {
                Some(serde_json::json!({
                    "agent_id": entry.id.to_string(),
                    "name": entry.name,
                    "daily_cost_usd": daily,
                    "hourly_limit": entry.manifest.resources.max_cost_per_hour_usd,
                    "daily_limit": entry.manifest.resources.max_cost_per_day_usd,
                    "monthly_limit": entry.manifest.resources.max_cost_per_month_usd,
                    "max_llm_tokens_per_hour": entry.manifest.resources.max_llm_tokens_per_hour,
                }))
            } else {
                None
            }
        })
        .collect();

    Json(serde_json::json!({"agents": agents, "total": agents.len()}))
}

/// PUT /api/budget/agents/{id} — Update per-agent budget limits at runtime.
pub async fn update_agent_budget(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let hourly = body["max_cost_per_hour_usd"].as_f64();
    let daily = body["max_cost_per_day_usd"].as_f64();
    let monthly = body["max_cost_per_month_usd"].as_f64();
    let tokens = body["max_llm_tokens_per_hour"].as_u64();

    if hourly.is_none() && daily.is_none() && monthly.is_none() && tokens.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour"}),
            ),
        );
    }

    match state
        .kernel
        .registry
        .update_resources(agent_id, hourly, daily, monthly, tokens)
    {
        Ok(()) => {
            // Persist updated entry
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Agent budget updated"})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// GET /api/sessions — List all sessions with metadata.
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.list_sessions() {
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
    Path(id): Path<String>,
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(_) => (), Err(r) => return r };

    // Parse the new manifest
    let _manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid manifest: {e}")})),
            );
        }
    };

    // Note: Full manifest update requires kill + respawn. For now, acknowledge receipt.
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "acknowledged",
            "agent_id": id,
            "note": "Full manifest update requires agent restart. Use DELETE + POST to apply.",
        })),
    )
}

/// PATCH /api/agents/{id} — Partial update of agent fields (name, description, model, system_prompt).
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(_) => (), Err(r) => return r };

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

// ── Model Catalog Endpoints ─────────────────────────────────────────

/// GET /api/models — List all models in the catalog.
///
/// Query parameters:
/// - `provider` — filter by provider (e.g. `?provider=anthropic`)
/// - `tier` — filter by tier (e.g. `?tier=smart`)
/// - `available` — only show models from configured providers (`?available=true`)
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let models: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .filter(|m| {
            if let Some(ref p) = provider_filter {
                if m.provider.to_lowercase() != *p {
                    return false;
                }
            }
            if let Some(ref t) = tier_filter {
                if m.tier.to_string() != *t {
                    return false;
                }
            }
            if available_only {
                let provider = catalog.get_provider(&m.provider);
                if let Some(p) = provider {
                    if p.auth_status == opencarrier_types::model_catalog::AuthStatus::Missing {
                        return false;
                    }
                }
            }
            true
        })
        .map(|m| {
            // Custom models from unknown providers are assumed available
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != opencarrier_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == opencarrier_types::model_catalog::ModelTier::Custom);
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": m.provider,
                "tier": m.tier,
                "context_window": m.context_window,
                "max_output_tokens": m.max_output_tokens,
                "input_cost_per_m": m.input_cost_per_m,
                "output_cost_per_m": m.output_cost_per_m,
                "supports_tools": m.supports_tools,
                "supports_vision": m.supports_vision,
                "supports_streaming": m.supports_streaming,
                "available": available,
            })
        })
        .collect();

    let total = catalog.list_models().len();
    let available_count = catalog.available_models().len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": models,
            "total": total,
            "available": available_count,
        })),
    )
}

/// GET /api/models/aliases — List all alias-to-model mappings.
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list_aliases()
        .clone();
    let entries: Vec<serde_json::Value> = aliases
        .iter()
        .map(|(alias, model_id)| {
            serde_json::json!({
                "alias": alias,
                "model_id": model_id,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "aliases": entries,
            "total": entries.len(),
        })),
    )
}

/// GET /api/models/{id} — Get a single model by ID or alias.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(m) => {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != opencarrier_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == opencarrier_types::model_catalog::ModelTier::Custom);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": m.id,
                    "display_name": m.display_name,
                    "provider": m.provider,
                    "tier": m.tier,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "input_cost_per_m": m.input_cost_per_m,
                    "output_cost_per_m": m.output_cost_per_m,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "supports_streaming": m.supports_streaming,
                    "aliases": m.aliases,
                    "available": available,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Model '{}' not found", id)})),
        ),
    }
}

/// GET /api/providers — List all providers with auth status.
///
/// For local providers (ollama, vllm, lmstudio), also probes reachability and
/// discovers available models via their health endpoints.
///
/// Probes run **concurrently** and results are **cached for 60 seconds** so the
/// endpoint responds instantly on repeated dashboard loads even when local
/// providers are unreachable (fixes #474).
pub async fn list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let provider_list: Vec<opencarrier_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    // Collect local providers that need probing
    let local_providers: Vec<(usize, String, String)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.key_required && !p.base_url.is_empty())
        .map(|(i, p)| (i, p.id.clone(), p.base_url.clone()))
        .collect();

    // Fire all probes concurrently (cached results return instantly)
    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url)| {
            opencarrier_runtime::provider_health::probe_provider_cached(id, url, cache)
        })
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    // Index probe results by provider list position for O(1) lookup
    let mut probe_map: HashMap<usize, opencarrier_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((idx, _, _), result) in local_providers.iter().zip(probe_results.into_iter()) {
        probe_map.insert(*idx, result);
    }

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());

    for (i, p) in provider_list.iter().enumerate() {
        let mut entry = serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "auth_status": p.auth_status,
            "model_count": p.model_count,
            "key_required": p.key_required,
            "api_key_env": p.api_key_env,
            "base_url": p.base_url,
        });

        // For local providers, attach the probe result
        if let Some(probe) = probe_map.remove(&i) {
            entry["is_local"] = serde_json::json!(true);
            entry["reachable"] = serde_json::json!(probe.reachable);
            entry["latency_ms"] = serde_json::json!(probe.latency_ms);
            if !probe.discovered_models.is_empty() {
                entry["discovered_models"] = serde_json::json!(probe.discovered_models);
                // Merge discovered models into the catalog so agents can use them
                if let Ok(mut catalog) = state.kernel.model_catalog.write() {
                    catalog.merge_discovered_models(&p.id, &probe.discovered_models);
                }
            }
            if let Some(err) = &probe.error {
                entry["error"] = serde_json::json!(err);
            }
        } else if !p.key_required {
            // Local provider with empty base_url (e.g. claude-code) — skip probing
            entry["is_local"] = serde_json::json!(true);
        }

        providers.push(entry);
    }

    let total = providers.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "providers": providers,
            "total": total,
        })),
    )
}

/// POST /api/models/custom — Add a custom model to the catalog.
///
/// Persists to `~/.opencarrier/custom_models.json` and makes the model immediately
/// available for agent assignment.
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openrouter")
        .to_string();
    let context_window = body
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: id"})),
        );
    }

    let display = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let entry = opencarrier_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: opencarrier_types::model_catalog::ModelTier::Custom,
        context_window,
        max_output_tokens: max_output,
        input_cost_per_m: body
            .get("input_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        output_cost_per_m: body
            .get("output_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        supports_tools: body
            .get("supports_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        supports_vision: body
            .get("supports_vision")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        supports_streaming: body
            .get("supports_streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        aliases: vec![],
    };

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"error": format!("Model '{}' already exists for provider '{}'", id, provider)}),
            ),
        );
    }

    // Persist to disk
    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "provider": provider,
            "status": "added"
        })),
    )
}

/// DELETE /api/models/custom/{id} — Remove a custom model.
pub async fn remove_custom_model(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_custom_model(&model_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Custom model '{}' not found", model_id)})),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
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
            None, // exec_policy
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path((id, session_id_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(_) => (), Err(r) => return r };
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };
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
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };
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
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };
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
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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

// ── Provider Key Management Endpoints ──────────────────────────────────

/// Write a key=value line to a secrets.env file.
/// Creates the file if it doesn't exist; updates the value if the key already exists.
fn write_secret_env(path: &std::path::Path, key: &str, value: &str) -> Result<(), String> {
    let existing = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read secrets.env: {e}"))?
    } else {
        String::new()
    };
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let pattern = format!("{key}=");
    let mut found = false;
    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.starts_with(&pattern) && !trimmed.starts_with('#') {
            *line = format!("{key}={value}");
            found = true;
            break;
        }
    }
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let content = lines.join("\n");
    std::fs::write(path, content).map_err(|e| format!("Failed to write secrets.env: {e}"))
}

/// Remove a key=value line from a secrets.env file.
fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(path).map_err(|e| format!("Failed to read secrets.env: {e}"))?;
    let pattern = format!("{key}=");
    let lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.starts_with(&pattern) || trimmed.starts_with('#')
        })
        .map(|l| l.to_string())
        .collect();
    let content = lines.join("\n");
    std::fs::write(path, content).map_err(|e| format!("Failed to write secrets.env: {e}"))
}

/// POST /api/providers/{name}/key — Save an API key for a provider.
///
/// SECURITY: Writes to `~/.opencarrier/secrets.env`, sets env var in process,
/// and refreshes auth detection. Key is zeroized after use.
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'key' field"})),
            );
        }
    };

    // Look up env var from catalog; for unknown/custom providers derive one.
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom provider — derive env var: MY_PROVIDER → MY_PROVIDER_API_KEY
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    // Write to secrets.env file
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_var, &key) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write secrets.env: {e}")})),
        );
    }

    // Set env var in current process so detect_auth picks it up
    std::env::set_var(&env_var, &key);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    // Auto-switch default provider if current default has no working key.
    // This fixes the common case where a user adds e.g. a Gemini key via dashboard
    // but their agent still tries to use the previous provider (which has no key).
    //
    // Read the effective default from the hot-reload override (if set) rather than
    // the stale boot-time config — a previous set_provider_key call may have already
    // switched the default.
    let (current_provider, current_key_env) = {
        let guard = state
            .kernel
            .default_model_override
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => (dm.provider.clone(), dm.api_key_env.clone()),
            None => (
                state.kernel.config.default_model.provider.clone(),
                state.kernel.config.default_model.api_key_env.clone(),
            ),
        }
    };
    let current_has_key = if current_key_env.is_empty() {
        false
    } else {
        std::env::var(&current_key_env)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
    };
    let switched = if !current_has_key && current_provider != name {
        // Find a default model for the newly-keyed provider
        let default_model = {
            let catalog = state
                .kernel
                .model_catalog
                .read()
                .unwrap_or_else(|e| e.into_inner());
            catalog.default_model_for_provider(&name)
        };
        if let Some(model_id) = default_model {
            // Update config.toml to persist the switch
            let config_path = state.kernel.config.home_dir.join("config.toml");
            let update_toml = format!(
                "\n[default_model]\nprovider = \"{}\"\nmodel = \"{}\"\napi_key_env = \"{}\"\n",
                name, model_id, env_var
            );
            backup_config(&config_path);
            if let Ok(existing) = std::fs::read_to_string(&config_path) {
                let cleaned = remove_toml_section(&existing, "default_model");
                let _ =
                    std::fs::write(&config_path, format!("{}\n{}", cleaned.trim(), update_toml));
            } else {
                let _ = std::fs::write(&config_path, update_toml);
            }

            // Hot-update the in-memory default model override so resolve_driver()
            // immediately creates drivers for the new provider — no restart needed.
            {
                let new_dm = opencarrier_types::config::DefaultModelConfig {
                    provider: name.clone(),
                    model: model_id,
                    api_key_env: env_var.clone(),
                    base_url: None,
                };
                let mut guard = state
                    .kernel
                    .default_model_override
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                *guard = Some(new_dm);
            }
            true
        } else {
            false
        }
    } else if current_provider == name {
        // User is saving a key for the CURRENT default provider. The env var is
        // already set (set_var above), but we must ensure default_model_override
        // has the correct api_key_env so resolve_driver reads the right variable.
        let needs_update = {
            let guard = state
                .kernel
                .default_model_override
                .read()
                .unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(dm) => dm.api_key_env != env_var,
                None => state.kernel.config.default_model.api_key_env != env_var,
            }
        };
        if needs_update {
            let mut guard = state
                .kernel
                .default_model_override
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let base = guard
                .clone()
                .unwrap_or_else(|| state.kernel.config.default_model.clone());
            *guard = Some(opencarrier_types::config::DefaultModelConfig {
                api_key_env: env_var.clone(),
                ..base
            });
        }
        false
    } else {
        false
    };

    let mut resp = serde_json::json!({"status": "saved", "provider": name});
    if switched {
        resp["switched_default"] = serde_json::json!(true);
        resp["message"] = serde_json::json!(format!(
            "API key saved and default provider switched to '{}'.",
            name
        ));
    }

    (StatusCode::OK, Json(resp))
}

/// DELETE /api/providers/{name}/key — Remove an API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom/unknown provider — derive env var from convention
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    if env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider does not require an API key"})),
        );
    }

    // Remove from secrets.env
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update secrets.env: {e}")})),
        );
    }

    // Remove from process environment
    std::env::remove_var(&env_var);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "provider": name})),
    )
}

/// POST /api/providers/{name}/test — Test a provider's connectivity.
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (env_var, base_url, key_required, default_model) = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => {
                // Find a default model for this provider to use in the test request
                let model_id = catalog
                    .default_model_for_provider(&name)
                    .unwrap_or_default();
                (
                    p.api_key_env.clone(),
                    p.base_url.clone(),
                    p.key_required,
                    model_id,
                )
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
                );
            }
        }
    };

    let api_key = std::env::var(&env_var).ok();
    // Only require API key for providers that need one (skip local providers like ollama/vllm/lmstudio)
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider API key not configured"})),
        );
    }

    // Attempt a lightweight connectivity test
    let start = std::time::Instant::now();
    let driver_config = opencarrier_runtime::llm_driver::DriverConfig {
        provider: name.clone(),
        api_key,
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
        skip_permissions: true,
    };

    match opencarrier_runtime::drivers::create_driver(&driver_config) {
        Ok(driver) => {
            // Send a minimal completion request to test connectivity
            let test_req = opencarrier_runtime::llm_driver::CompletionRequest {
                model: default_model.clone(),
                messages: vec![opencarrier_types::message::Message::user("Hi")],
                tools: vec![],
                max_tokens: 1,
                temperature: 0.0,
                system: None,
                thinking: None,
            };
            match driver.complete(test_req).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "ok",
                            "provider": name,
                            "latency_ms": latency_ms,
                        })),
                    )
                }
                Err(e) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "provider": name,
                        "error": format!("{e}"),
                    })),
                ),
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Failed to create driver: {e}"),
            })),
        ),
    }
}

/// PUT /api/providers/{name}/url — Set a custom base URL for a provider.
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Accept any provider name — custom providers are supported via OpenAI-compatible format.
    let base_url = match body["base_url"].as_str() {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'base_url' field"})),
            );
        }
    };

    // Validate URL scheme
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base_url must start with http:// or https://"})),
        );
    }

    // Update catalog in memory
    {
        let mut catalog = state
            .kernel
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
    }

    // Persist to config.toml [provider_urls] section
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        );
    }

    // Probe reachability at the new URL
    let probe = opencarrier_runtime::provider_health::probe_provider(&name, &base_url).await;

    // Merge discovered models into catalog
    if !probe.discovered_models.is_empty() {
        if let Ok(mut catalog) = state.kernel.model_catalog.write() {
            catalog.merge_discovered_models(&name, &probe.discovered_models);
        }
    }

    let mut resp = serde_json::json!({
        "status": "saved",
        "provider": name,
        "base_url": base_url,
        "reachable": probe.reachable,
        "latency_ms": probe.latency_ms,
    });
    if !probe.discovered_models.is_empty() {
        resp["discovered_models"] = serde_json::json!(probe.discovered_models);
    }

    (StatusCode::OK, Json(resp))
}

/// Upsert a provider URL in the `[provider_urls]` section of config.toml.
fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_urls") {
        root.insert(
            "provider_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let urls_table = root
        .get_mut("provider_urls")
        .and_then(|v| v.as_table_mut())
        .ok_or("provider_urls is not a table")?;

    urls_table.insert(provider.to_string(), toml::Value::String(url.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

/// POST /api/skills/create — Create a local prompt-only skill.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
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
    Path(id): Path<String>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

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
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    Path((id, filename)): Path<(String, String)>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
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
    #[allow(dead_code)]
    filename: String,
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
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Validate agent ID format
    let _agent_id = match parse_agent_id(&id) {
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
            filename: filename.clone(),
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
pub async fn config_reload(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
pub async fn config_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'path' field"})),
            );
        }
    };
    let value = match body.get("value") {
        Some(v) => v.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'value' field"})),
            );
        }
    };

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
            );
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
            );
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        );
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
    )
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
// Delivery tracking endpoints
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/deliveries — List recent delivery receipts for an agent.
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&id) {
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

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let receipts = state.kernel.delivery_tracker.get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                state.kernel.cron_scheduler.list_jobs(aid)
            }
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid agent_id"})),
                );
            }
        }
    } else {
        state.kernel.cron_scheduler.list_all_jobs()
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
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
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
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
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
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
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
            // No agent specified — use the first available agent
            match state.kernel.registry.list().first() {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "No agents available"})),
                    );
                }
            }
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
pub async fn comms_topology(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use opencarrier_types::comms::{EdgeKind, TopoEdge, TopoNode, Topology};

    let agents = state.kernel.registry.list();

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
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);

    let agents = state.kernel.registry.list();

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
pub async fn comms_events_stream(State(state): State<Arc<AppState>>) -> axum::response::Response {
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
    Json(req): Json<opencarrier_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    // Validate from agent exists
    let from_id = match parse_agent_id(&req.from_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if state.kernel.registry.get(from_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Source agent not found"})),
        );
    }

    // Validate to agent exists
    let to_id = match parse_agent_id(&req.to_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if state.kernel.registry.get(to_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Target agent not found"})),
        );
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
    Json(req): Json<opencarrier_types::comms::CommsTaskRequest>,
) -> impl IntoResponse {
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

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    let token =
        crate::session_auth::create_session_token(username, &secret, auth_cfg.session_ttl_hours);
    let ttl_secs = auth_cfg.session_ttl_hours * 3600;
    let cookie = format!(
        "opencarrier_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}"
    );

    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::AuthAttempt,
        "dashboard login success",
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

    if let Some(username) = session_user {
        Json(serde_json::json!({
            "authenticated": true,
            "mode": "session",
            "username": username,
        }))
    } else {
        Json(serde_json::json!({
            "authenticated": false,
            "mode": "session",
        }))
    }
}

fn backup_config(config_path: &std::path::Path) {
    let backup = config_path.with_extension("toml.bak");
    let _ = std::fs::copy(config_path, backup);
}

fn remove_toml_section(content: &str, section: &str) -> String {
    let header = format!("[{}]", section);
    let mut result = String::new();
    let mut skipping = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

// ========== Clone (.agx) endpoints ==========

/// POST /api/clones/install — Install a .agx clone from uploaded bytes.
pub async fn install_clone(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InstallCloneRequest>,
) -> impl IntoResponse {
    use opencarrier_clone::{load_agx, install_clone_to_workspace, convert_to_manifest};

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

    // Write uploaded bytes to temp file
    let tmp_dir = std::env::temp_dir().join(format!("opencarrier-clone-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
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

    // Create workspace directory
    let workspace_dir = state.kernel.config.effective_workspaces_dir().join(&clone_data.name);
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

    // Spawn agent
    let name = manifest.name.clone();
    let warnings = clone_data.security_warnings.clone();

    match state.kernel.spawn_agent(manifest) {
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
) -> impl IntoResponse {
    let agents = state.kernel.registry.list();
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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"})));
        }
    };

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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"})));
        }
    };

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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Clone not found"})));
        }
    };

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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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
    Path(name): Path<String>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let filename = match body["filename"].as_str() {
        Some(f) => f.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'filename' field"})),
            )
        }
    };

    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let filename = match body["filename"].as_str() {
        Some(f) => f.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'filename' field"})),
            )
        }
    };

    let entry = match state.kernel.registry.find_by_name(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
            )
        }
    };

    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
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


