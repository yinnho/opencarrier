//! Agent key-value storage endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
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



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/memory/agents/{id}/kv", routing::get(get_agent_kv))
        .route("/api/memory/agents/{id}/kv/{key}", routing::put(set_agent_kv_key).delete(delete_agent_kv_key).get(get_agent_kv_key))
}
