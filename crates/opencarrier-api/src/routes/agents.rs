//! Agent lifecycle management endpoints.

use crate::routes::common::*;
use crate::routes::state::AppState;
use crate::types::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_types::agent::{AgentIdentity, AgentManifest};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Request types
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

/// Request body for cloning an agent.
#[derive(serde::Deserialize)]
pub struct CloneAgentRequest {
    pub new_name: String,
}

/// Request body for writing a knowledge file to an agent's workspace.
#[derive(serde::Deserialize)]
pub struct WriteKnowledgeRequest {
    pub filename: String,
    pub content: String,
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
    // Determine target tenant: admin can override, otherwise use context
    let target_tenant = if ctx.is_admin() {
        req.tenant_id.clone().or_else(|| ctx.tenant_id.clone())
    } else {
        ctx.tenant_id.clone()
    };
    let target_tenant_str = match target_tenant {
        Some(ref tid) => tid.as_str(),
        None => "",
    };
    match state.kernel.spawn_agent(manifest, target_tenant_str) {
        Ok(id) => {
            if let Some(tid) = target_tenant {
                state.kernel.registry.set_tenant_id(id, tid);
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
                "model_provider": modality,
                "model_name": model,
                "ready": ready,
                "profile": e.manifest.profile,
                "tenant_id": e.tenant_id,
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
    let (agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx)
    {
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
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------

/// GET /api/agents/:id — Get a single agent's detailed info.
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) =
        match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
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
// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/agents/:id — Update an agent (currently: re-set manifest fields).
/// PATCH /api/agents/{id} — Partial update of agent fields (name, description, model, system_prompt).
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (agent_id, _entry) =
        match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
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
        if let Err(e) = state.kernel.set_agent_model(agent_id, model) {
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

/// POST /api/agents/{id}/knowledge — Write a knowledge file to an agent's workspace.
pub async fn write_agent_knowledge(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<WriteKnowledgeRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let workspace = match &entry.manifest.workspace {
        Some(ws) => ws.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            )
        }
    };

    // Sanitize filename: only allow alphanumeric, hyphens, underscores, and .md extension
    let safe_name: String = body
        .filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    if safe_name.is_empty()
        || safe_name != body.filename
        || !safe_name.ends_with(".md")
        || safe_name.contains("..")
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid filename: use alphanumeric + .md only"})),
        );
    }

    let knowledge_dir = workspace.join("data").join("knowledge");
    let file_path: PathBuf = knowledge_dir.join(&safe_name);

    // Security: ensure resolved path is still under knowledge dir
    if let Ok(canonical_parent) = knowledge_dir.canonicalize() {
        if let Ok(canonical_file) = file_path.canonicalize() {
            if !canonical_file.starts_with(&canonical_parent) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Path traversal denied"})),
                );
            }
        }
    }

    if let Err(e) = std::fs::create_dir_all(&knowledge_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create knowledge dir: {e}")})),
        );
    }
    if let Err(e) = std::fs::write(&file_path, &body.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write knowledge file: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "filename": safe_name})),
    )
}

/// POST /api/agents/{id}/suspend — Suspend an agent (keep DB + workspace, stop processing).
pub async fn suspend_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    if let Err(e) = state
        .kernel
        .registry
        .set_state(agent_id, opencarrier_types::agent::AgentState::Suspended)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        );
    }
    if let Some(updated) = state.kernel.registry.get(agent_id) {
        let _ = state.kernel.memory.save_agent(&updated);
    }
    // Cancel any active run
    let _ = state.kernel.stop_agent_run(agent_id);
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "suspended", "name": entry.name})),
    )
}

/// POST /api/agents/{id}/resume — Resume a suspended agent.
pub async fn resume_agent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    if let Err(e) = state
        .kernel
        .registry
        .set_state(agent_id, opencarrier_types::agent::AgentState::Running)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        );
    }
    if let Some(updated) = state.kernel.registry.get(agent_id) {
        let _ = state.kernel.memory.save_agent(&updated);
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "running", "name": entry.name})),
    )
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
    match state.kernel.set_agent_model(agent_id, model) {
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

    let source = match get_agent_or_404(&state.kernel.registry, &agent_id) {
        Ok(e) => e,
        Err(r) => return r,
    };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Spawn the cloned agent — inherit source agent's tenant
    let target_tenant_str = source.tenant_id.as_str();
    let new_id = match state.kernel.spawn_agent(cloned_manifest, target_tenant_str) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Clone spawn failed: {e}")})),
            );
        }
    };

    // Assign tenant ownership — clone inherits source agent's tenant
    if let Some(tid) = ctx.tenant_id.clone() {
        state.kernel.registry.set_tenant_id(new_id, tid);
    }

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

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/agents", routing::post(spawn_agent).get(list_agents))
        .route(
            "/api/agents/{id}",
            routing::delete(kill_agent)
                .patch(patch_agent)
                .get(get_agent),
        )
        .route("/api/agents/{id}/clone", routing::post(clone_agent))
        .route(
            "/api/agents/{id}/config",
            routing::patch(patch_agent_config),
        )
        .route(
            "/api/agents/{id}/identity",
            routing::patch(update_agent_identity),
        )
        .route("/api/agents/{id}/mode", routing::put(set_agent_mode))
        .route("/api/agents/{id}/model", routing::put(set_model))
        .route("/api/agents/{id}/restart", routing::post(restart_agent))
        .route("/api/agents/{id}/start", routing::post(restart_agent))
        .route("/api/agents/{id}/stop", routing::post(stop_agent))
        .route("/api/agents/{id}/suspend", routing::post(suspend_agent))
        .route("/api/agents/{id}/resume", routing::post(resume_agent))
        .route("/api/agents/{id}/knowledge", routing::post(write_agent_knowledge))
}
