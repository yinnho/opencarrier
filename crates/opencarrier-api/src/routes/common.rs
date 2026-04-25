//! Shared helpers used by multiple route handlers.

use axum::http::StatusCode;
use axum::Json;
use opencarrier_types::agent::{AgentId, AgentEntry};

/// Parse a path-parameter agent ID, returning BAD_REQUEST on failure.
pub fn parse_agent_id(id: &str) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        )
    })
}

/// Parse agent ID, look up agent, and check tenant ownership.
/// Returns just the AgentId on success (for handlers that don't need the entry).
pub fn parse_agent_id_with_tenant(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    let (agent_id, _entry) = parse_and_get_agent_with_tenant(id, registry, ctx)?;
    Ok(agent_id)
}

/// Look up an agent in the registry, returning NOT_FOUND if missing.
pub fn get_agent_or_404(
    registry: &opencarrier_kernel::registry::AgentRegistry,
    agent_id: &AgentId,
) -> Result<AgentEntry, (StatusCode, Json<serde_json::Value>)> {
    registry.get(*agent_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        )
    })
}

/// Parse agent ID from path and look up the agent. Returns (AgentId, AgentEntry) or an error response.
pub fn parse_and_get_agent(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
) -> Result<(AgentId, AgentEntry), (StatusCode, Json<serde_json::Value>)> {
    let agent_id = parse_agent_id(id)?;
    let entry = get_agent_or_404(registry, &agent_id)?;
    Ok((agent_id, entry))
}

/// Parse agent ID, get agent entry, and check tenant ownership.
/// Returns 403 if the requester doesn't own the agent.
pub fn parse_and_get_agent_with_tenant(
    id: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<(AgentId, AgentEntry), (StatusCode, Json<serde_json::Value>)> {
    let (agent_id, entry) = parse_and_get_agent(id, registry)?;
    if !can_access(ctx, entry.tenant_id.as_deref()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
        ));
    }
    Ok((agent_id, entry))
}

/// Resolve an agent by UUID or name with tenant-aware access control.
///
/// - UUID: look up by ID, then check `can_access`.
/// - Name: look up scoped to the caller's tenant (or globally for admin),
///   then check `can_access`.
/// - Returns 403 if the caller can't access the resolved agent.
pub fn resolve_agent_id_with_tenant(
    id_or_name: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<(AgentId, AgentEntry), (StatusCode, Json<serde_json::Value>)> {
    // Try UUID first
    if let Ok(id) = id_or_name.parse::<AgentId>() {
        let entry = get_agent_or_404(registry, &id)?;
        if !can_access(ctx, entry.tenant_id.as_deref()) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
            ));
        }
        return Ok((id, entry));
    }
    // Name lookup — admin gets global, tenant gets scoped
    let entry = if ctx.is_admin() {
        registry.find_by_name(id_or_name)
    } else {
        ctx.tenant_id.as_ref().and_then(|tid| {
            registry.find_by_name_and_tenant(id_or_name, Some(tid.as_str()))
        })
    }.ok_or_else(|| (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("Agent not found: {id_or_name}")})),
    ))?;
    if !can_access(ctx, entry.tenant_id.as_deref()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
        ));
    }
    Ok((entry.id, entry))
}

/// Look up a clone by name and extract its workspace path.
/// Returns (AgentEntry, PathBuf) or an error response.
/// Uses global find_by_name — prefer `get_clone_workspace_with_tenant` for multi-tenant.
pub fn get_clone_workspace(
    name: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
) -> Result<(AgentEntry, std::path::PathBuf), (StatusCode, Json<serde_json::Value>)> {
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
/// Uses tenant-scoped lookup to avoid cross-tenant name collisions.
pub fn get_clone_workspace_with_tenant(
    name: &str,
    registry: &opencarrier_kernel::registry::AgentRegistry,
    ctx: &opencarrier_types::tenant::TenantContext,
) -> Result<(AgentEntry, std::path::PathBuf), (StatusCode, Json<serde_json::Value>)> {
    let entry = if ctx.is_admin() {
        registry.find_by_name(name)
    } else {
        ctx.tenant_id.as_ref().and_then(|tid| {
            registry.find_by_name_and_tenant(name, Some(tid.as_str()))
        })
    }.ok_or_else(|| (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("Clone '{name}' not found")})),
    ))?;
    if !can_access(ctx, entry.tenant_id.as_deref()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Access denied: resource belongs to another tenant"})),
        ));
    }
    let workspace = entry.manifest.workspace.clone().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Agent has no workspace"})),
        )
    })?;
    Ok((entry, workspace))
}

/// Helper: extract TenantContext from request extensions, defaulting to deny_all.
pub fn get_tenant_ctx(extensions: &axum::http::Extensions) -> opencarrier_types::tenant::TenantContext {
    extensions
        .get::<opencarrier_types::tenant::TenantContext>()
        .cloned()
        .unwrap_or_else(opencarrier_types::tenant::TenantContext::deny_all)
}

/// Helper: check if the requester can access a resource owned by `resource_tenant_id`.
/// Admin can access everything. Tenants can only access their own resources.
pub fn can_access(ctx: &opencarrier_types::tenant::TenantContext, resource_tenant_id: Option<&str>) -> bool {
    if ctx.is_admin() {
        return true;
    }
    match (&ctx.tenant_id, resource_tenant_id) {
        (Some(tid), Some(rid)) => tid == rid,
        (Some(_), None) => false, // tenant can't access global resources
        (None, _) => false,        // deny — missing tenant context is not admin
    }
}

// ---------------------------------------------------------------------------
// Shared upload registry (used by files, messaging, and sessions modules)
// ---------------------------------------------------------------------------

use dashmap::DashMap;
use std::sync::LazyLock;

/// Metadata stored alongside uploaded files.
pub struct UploadMeta {
    pub content_type: String,
    pub tenant_id: Option<String>,
}

/// In-memory upload metadata registry.
pub static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

// ---------------------------------------------------------------------------
// Workspace identity file whitelist (used by agents and files modules)
// ---------------------------------------------------------------------------

/// Immutable identity files — can be created but never overwritten via the API.
pub const IMMUTABLE_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
];

/// Whitelisted workspace identity files that can be read/written via API.
pub const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];
