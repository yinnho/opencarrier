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
pub async fn list_plugins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut result = Vec::new();

    // Collect loaded plugins from runtime
    let mut loaded_names = std::collections::HashSet::new();
    let plugins_dir = match &state.kernel.config.plugins_dir {
        Some(dir) => dir.clone(),
        None => state.kernel.config.home_dir.join("plugins"),
    };

    if let Some(ref pm) = state.plugin_manager {
        let pm = pm.lock().await;
        let statuses = pm.status();
        for s in &statuses {
            loaded_names.insert(s.name.clone());
            // If runtime channels are empty, fall back to plugin.toml declaration
            let channels = if s.channels.is_empty() {
                let toml_path = plugins_dir.join(&s.name).join("plugin.toml");
                read_channels_from_toml(&toml_path)
            } else {
                s.channels.clone()
            };
            result.push(serde_json::json!({
                "name": s.name,
                "version": s.version,
                "loaded": true,
                "channels": channels,
                "tools": s.tools,
            }));
        }
    }

    // Scan filesystem for installed-but-not-loaded plugins
    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if loaded_names.contains(&name) {
                continue;
            }
            let toml_path = entry.path().join("plugin.toml");
            let (channels, version) = read_toml_meta(&toml_path);
            result.push(serde_json::json!({
                "name": name,
                "version": version,
                "loaded": false,
                "channels": channels,
                "tools": [],
            }));
        }
    }

    Json(serde_json::json!({
        "plugins": result,
        "count": result.len(),
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
                Json(
                    serde_json::json!({"error": format!("Hub API key not set (env: {})", api_key_env)}),
                ),
            );
        }
    };

    match opencarrier_clone::hub::install_plugin(
        &hub_url,
        &api_key,
        &body.name,
        body.version.as_deref(),
        &plugins_dir,
    )
    .await
    {
        Ok(name) => (
            axum::http::StatusCode::OK,
            Json(
                serde_json::json!({"ok": true, "name": name, "message": "Plugin installed. Restart daemon to load."}),
            ),
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
            Json(
                serde_json::json!({"ok": true, "message": format!("Plugin '{}' removed. Restart daemon to unload.", name)}),
            ),
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
    let api_key = std::env::var(api_key_env).unwrap_or_default();

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

fn read_channels_from_toml(path: &std::path::Path) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let doc = match content.parse::<toml::Value>() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    doc.get("channels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("channel_type").and_then(|ct| ct.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn read_toml_meta(path: &std::path::Path) -> (Vec<String>, String) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (vec![], "0.1.0".to_string()),
    };
    let doc = match content.parse::<toml::Value>() {
        Ok(d) => d,
        Err(_) => return (vec![], "0.1.0".to_string()),
    };
    let channels = doc.get("channels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("channel_type").and_then(|ct| ct.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let version = doc.get("plugin")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0")
        .to_string();
    (channels, version)
}
