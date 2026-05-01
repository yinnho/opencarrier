//! Brain configuration and management endpoints.

use crate::routes::common::*;
use crate::routes::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
/// GET /api/brain — Brain configuration and status.
///
/// Returns the Brain's modalities, endpoints, and which ones are ready.
pub async fn brain_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let brain = state.kernel.brain_info();
    let config = brain.config();
    let ready = brain.ready_endpoints();

    let mut endpoints = serde_json::Map::new();
    for (name, ep) in &config.endpoints {
        endpoints.insert(
            name.clone(),
            serde_json::json!({
                "provider": ep.provider,
                "model": ep.model,
                "base_url": ep.base_url,
                "format": ep.format.to_string(),
                "ready": ready.contains(name),
            }),
        );
    }

    let mut modalities = serde_json::Map::new();
    for (name, mc) in &config.modalities {
        modalities.insert(
            name.clone(),
            serde_json::json!({
                "primary": mc.primary,
                "fallbacks": mc.fallbacks,
            }),
        );
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
    let api_key_env = body["api_key_env"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    let result = state.kernel.update_brain(|config| {
        config.providers.insert(
            name.clone(),
            opencarrier_types::brain::ProviderConfig {
                api_key_env,
                auth_type: "apikey".to_string(),
                params: std::collections::HashMap::new(),
            },
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
    let provider = body["provider"].as_str().unwrap_or("").trim().to_string();
    let model = body["model"].as_str().unwrap_or("").trim().to_string();
    let base_url = body["base_url"].as_str().unwrap_or("").trim().to_string();
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
                Json(
                    serde_json::json!({"error": "format must be 'openai', 'anthropic', or 'gemini'"}),
                ),
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
    let primary = body["primary"].as_str().unwrap_or("").trim().to_string();
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
pub async fn reload_brain(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
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
pub async fn get_brain_config_raw(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
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
    let path = state.kernel.brain_path();
    match std::fs::read_to_string(path) {
        Ok(json_str) => match serde_json::from_str::<serde_json::Value>(&json_str) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(_) => (StatusCode::OK, Json(serde_json::json!({"_raw": json_str}))),
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
    {
        let ctx = get_tenant_ctx(&extensions);
        if !ctx.is_admin() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Admin only"})),
            );
        }
    }
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
                if reload_result.is_ok() {
                    "ok"
                } else {
                    "reload_failed"
                },
            );
            match reload_result {
                Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
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

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/brain", routing::get(brain_info))
        .route(
            "/api/brain/config",
            routing::put(put_brain_config_raw).get(get_brain_config_raw),
        )
        .route(
            "/api/brain/default-modality",
            routing::put(set_brain_default_modality),
        )
        .route(
            "/api/brain/endpoints/{name}",
            routing::delete(delete_brain_endpoint).put(set_brain_endpoint),
        )
        .route(
            "/api/brain/modalities/{name}",
            routing::delete(delete_brain_modality)
                .get(brain_modality_detail)
                .put(set_brain_modality),
        )
        .route(
            "/api/brain/providers/{name}",
            routing::delete(delete_brain_provider).put(set_brain_provider),
        )
        .route("/api/brain/reload", routing::post(reload_brain))
        .route("/api/brain/status", routing::get(brain_status))
}
