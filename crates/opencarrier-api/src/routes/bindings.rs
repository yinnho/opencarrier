//! Agent binding endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
// ─── Agent Bindings API ────────────────────────────────────────────────

/// GET /api/bindings — List all agent bindings.
pub async fn list_bindings(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let bindings = state.kernel.list_bindings();
    // Filter: tenants can only see bindings for their agents
    let filtered: Vec<_> = if ctx.is_admin() {
        bindings
    } else {
        bindings.into_iter().filter(|b| {
            // Check if the binding's agent belongs to this tenant
            if let Ok(uuid) = b.agent.parse::<uuid::Uuid>() {
                if let Some(entry) = state.kernel.registry.get(opencarrier_types::agent::AgentId(uuid)) {
                    return can_access(&ctx, entry.tenant_id.as_str());
                }
            }
            // Name lookup scoped to tenant
            ctx.tenant_id.as_ref().map(|tid| {
                state.kernel.registry.find_by_name_and_tenant(&b.agent, tid.as_str()).is_some()
            }).unwrap_or(false)
        }).collect()
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": filtered })),
    )
}
/// POST /api/bindings — Add a new agent binding.
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(binding): Json<opencarrier_types::config::AgentBinding>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);

    // Tenant check: verify the binding's agent belongs to the caller
    let agent_owned = if let Ok(uuid) = binding.agent.parse::<uuid::Uuid>() {
        state.kernel.registry.get(opencarrier_types::agent::AgentId(uuid))
            .map(|e| can_access(&ctx, &e.tenant_id))
            .unwrap_or(false)
    } else {
        ctx.tenant_id.as_ref().map(|tid| {
            state.kernel.registry.find_by_name_and_tenant(&binding.agent, tid.as_str()).is_some()
        }).unwrap_or(false)
    };
    if !agent_owned && !ctx.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Cannot bind to an agent you don't own"})),
        );
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
    extensions: axum::http::Extensions,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    // Verify ownership of the binding being removed
    let bindings = state.kernel.list_bindings();
    if let Some(binding) = bindings.get(index) {
        let agent_owned = if let Ok(uuid) = binding.agent.parse::<uuid::Uuid>() {
            state.kernel.registry.get(opencarrier_types::agent::AgentId(uuid))
                .map(|e| can_access(&ctx, &e.tenant_id))
                .unwrap_or(false)
        } else {
            ctx.tenant_id.as_ref().map(|tid| {
                state.kernel.registry.find_by_name_and_tenant(&binding.agent, tid.as_str()).is_some()
            }).unwrap_or(false)
        };
        if !agent_owned && !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Cannot remove a binding for an agent you don't own"})),
            );
        }
    } else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Binding index out of range"})),
        );
    }

    match state.kernel.remove_binding(index) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Binding index out of range"})),
        ),
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/bindings", routing::post(add_binding).get(list_bindings))
        .route("/api/bindings/{index}", routing::delete(remove_binding))
}
