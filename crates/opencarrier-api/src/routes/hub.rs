//! Hub template marketplace endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_runtime::kernel_handle::KernelHandle;
use std::sync::Arc;
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
/// Body (optional): `{ "tenant_id": "..." }` — admin can specify target tenant.
pub async fn install_hub_template(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let target_tenant: Option<String> = if ctx.is_admin() {
        body.get("tenant_id").and_then(|v| v.as_str().map(String::from))
            .or_else(|| ctx.tenant_id.clone())
    } else {
        ctx.tenant_id.clone()
    };
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

    let device_id = opencarrier_clone::hub::get_or_create_device_id(&state.kernel.config.home_dir)
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
        tracing::warn!(%status, %body, key_prefix = &hub_api_key[..8.min(hub_api_key.len())], "Hub download failed");
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

    match state.kernel.clone_install(&name, &agx_bytes, target_tenant.as_deref()).await {
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



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/hub/templates", routing::get(list_hub_templates))
        .route("/api/hub/templates/{name}/install", routing::post(install_hub_template))
}
