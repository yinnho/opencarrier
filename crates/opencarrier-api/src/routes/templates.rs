//! Template listing and retrieval endpoints.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_types::agent::AgentManifest;
// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
pub async fn list_templates() -> impl IntoResponse {
    let agents_dir = opencarrier_kernel::config::opencarrier_home().join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("agent.toml");
                if manifest_path.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let description = std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                        .map(|m| m.description)
                        .unwrap_or_default();

                    templates.push(serde_json::json!({
                        "name": name,
                        "description": description,
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}
/// GET /api/templates/:name — Get template details.
pub async fn get_template(Path(name): Path<String>) -> impl IntoResponse {
    // Reject path traversal attempts
    if name.contains('.') || name.contains(std::path::MAIN_SEPARATOR) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid template name"})),
        );
    }

    let agents_dir = opencarrier_kernel::config::opencarrier_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    // Verify resolved path stays within agents_dir
    if let (Ok(canonical_agents), Ok(canonical_manifest)) =
        (agents_dir.canonicalize(), manifest_path.canonicalize())
    {
        if !canonical_manifest.starts_with(&canonical_agents) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid template name"})),
            );
        }
    }

    if !manifest_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Template not found"})),
        );
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "modality": manifest.model.modality,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Invalid template manifest"})),
                )
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read template"})),
            )
        }
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/templates", routing::get(list_templates))
        .route("/api/templates/{name}", routing::get(get_template))
}
