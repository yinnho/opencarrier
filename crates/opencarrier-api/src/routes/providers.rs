//! Provider API key management endpoints.

use crate::routes::common::*;
use crate::routes::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
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
                pc.params
                    .values()
                    .all(|env_name| opencarrier_kernel::dotenv::has_env_key(env_name))
            } else {
                opencarrier_kernel::dotenv::has_env_key(&pc.api_key_env)
            };

            let params_status: Vec<serde_json::Value> = pc
                .params
                .iter()
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
        if reload_result.is_ok() {
            "ok"
        } else {
            "reload_failed"
        },
    );
    match reload_result {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "warning": format!("Key saved but brain reload failed: {}", e)}),
            ),
        ),
    }
}
/// DELETE /api/providers/{name}/key — Remove API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
) -> impl IntoResponse {
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/providers/keys", routing::get(list_provider_keys))
        .route(
            "/api/providers/{name}/key",
            routing::delete(delete_provider_key).post(set_provider_key),
        )
}
