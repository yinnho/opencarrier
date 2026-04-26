//! Plugin management routes — list, install, remove, search.

use crate::routes::state::AppState;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/plugins", routing::get(list_plugins))
        .route("/api/plugins/install", routing::post(install_plugin))
        .route("/api/plugins/search", routing::get(search_plugins))
        .route("/api/plugins/{name}", routing::delete(remove_plugin))
}

/// List installed plugins.
pub async fn list_plugins(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
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
        let guard = state.kernel.plugins.plugin_tool_dispatcher.lock().unwrap();
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

#[derive(Deserialize)]
pub struct InstallRequest {
    name: String,
    #[allow(dead_code)]
    version: Option<String>,
}

/// Install a plugin from Hub.
pub async fn install_plugin(
    State(state): State<Arc<AppState>>,
    Json(body): Json<InstallRequest>,
) -> impl IntoResponse {
    let config = &state.kernel.config;
    let plugins_dir = match &config.plugins_dir {
        Some(dir) => dir.clone(),
        None => config.home_dir.join("plugins"),
    };

    let hub_url = config.hub.url.trim_end_matches('/').to_string();
    let api_key_env = &config.hub.api_key_env;
    let api_key = match std::env::var(api_key_env) {
        Ok(k) => k,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Hub API key not set (env: {})", api_key_env)})),
            );
        }
    };

    // Check if already installed
    if opencarrier_clone::hub::is_plugin_installed(&plugins_dir, &body.name) {
        return (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"ok": true, "message": format!("Plugin '{}' already installed", body.name)})),
        );
    }

    match opencarrier_clone::hub::install_plugin(
        &hub_url,
        &api_key,
        &body.name,
        body.version.as_deref(),
        &plugins_dir,
    ).await {
        Ok(name) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"ok": true, "name": name, "message": "Plugin installed. Restart daemon to load."})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to install plugin: {e}")})),
        ),
    }
}

/// Remove a plugin.
pub async fn remove_plugin(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let config = &state.kernel.config;
    let plugins_dir = match &config.plugins_dir {
        Some(dir) => dir.clone(),
        None => config.home_dir.join("plugins"),
    };

    let plugin_dir = plugins_dir.join(&name);
    if !plugin_dir.is_dir() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Plugin '{}' not found", name)})),
        );
    }

    // Security: ensure the path doesn't escape plugins_dir
    if !plugin_dir.starts_with(&plugins_dir) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid plugin name"})),
        );
    }

    match std::fs::remove_dir_all(&plugin_dir) {
        Ok(_) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"ok": true, "message": format!("Plugin '{}' removed. Restart daemon to unload.", name)})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to remove plugin: {e}")})),
        ),
    }
}

/// Search plugins on Hub.
pub async fn search_plugins(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let config = &state.kernel.config;
    let hub_url = config.hub.url.trim_end_matches('/').to_string();
    let api_key_env = &config.hub.api_key_env;
    let api_key = match std::env::var(api_key_env) {
        Ok(k) => k,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Hub API key not set (env: {})", api_key_env)})),
            );
        }
    };

    let query = params.get("q").map(|s| s.as_str()).unwrap_or("");

    match opencarrier_clone::hub::search_plugins(&hub_url, &api_key, query).await {
        Ok(result) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"result": result})),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Hub search failed: {e}")})),
        ),
    }
}
