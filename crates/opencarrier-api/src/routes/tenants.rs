//! Tenant management endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_types::agent::AgentManifest;
use std::sync::Arc;
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
                if state.kernel.registry.find_by_name_and_tenant(clone_name, Some(&tenant_id)).is_some() {
                    // Already running in this tenant — skip
                    continue;
                }
                // Check if the clone is installed on disk (tenant-scoped path)
                let clone_toml = state
                    .kernel
                    .config
                    .tenant_workspaces_dir(Some(tenant_id.as_str()))
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



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/tenants", routing::post(create_tenant).get(list_tenants))
        .route("/api/tenants/{id}", routing::put(update_tenant).delete(delete_tenant).get(get_tenant))
}
