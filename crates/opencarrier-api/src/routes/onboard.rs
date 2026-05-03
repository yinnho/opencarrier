//! Onboarding API — public endpoint for share page to create tenants and default agents.

use crate::routes::state::AppState;
use crate::session_auth;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_types::agent::AgentManifest;
use std::sync::Arc;

pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/onboard", routing::post(onboard))
}

#[derive(serde::Deserialize)]
pub struct OnboardRequest {
    #[allow(dead_code)]
    source: Option<String>,
}

/// POST /api/onboard — create a new tenant with default agents.
/// Public endpoint (no auth required).
pub async fn onboard(
    State(state): State<Arc<AppState>>,
    Json(_req): Json<OnboardRequest>,
) -> impl IntoResponse {
    let suffix = &uuid::Uuid::new_v4().to_string()[..8];
    let tenant_name = format!("user_{suffix}");
    let tenant_password = uuid::Uuid::new_v4().to_string();

    let now = chrono::Utc::now().to_rfc3339();
    let password_hash = session_auth::hash_password(&tenant_password);
    let tenant_id = uuid::Uuid::new_v4().to_string();

    let entry = opencarrier_types::tenant::TenantEntry {
        id: tenant_id.clone(),
        name: tenant_name.clone(),
        password_hash,
        role: opencarrier_types::tenant::TenantRole::Tenant,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };

    let tenant_store = state.kernel.memory.tenant();
    if let Err(e) = tenant_store.create_tenant(&entry) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create tenant: {e}")})),
        );
    }

    // Spawn default agents (clone-creator, clone-trainer) from existing templates
    for agent_name in &["clone-creator", "clone-trainer"] {
        let manifest = find_agent_template(&state, agent_name);
        let manifest = match manifest {
            Some(m) => m,
            None => {
                tracing::warn!("No {} template found, skipping", agent_name);
                continue;
            }
        };

        match state.kernel.spawn_agent(manifest, &tenant_id) {
            Ok(agent_id) => {
                state
                    .kernel
                    .registry
                    .set_tenant_id(agent_id, tenant_id.clone());
                tracing::info!("Onboard spawned {} for tenant {}", agent_name, tenant_name);
            }
            Err(e) => {
                tracing::warn!("Failed to spawn {} for {}: {}", agent_name, tenant_name, e);
            }
        }
    }

    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        state.kernel.config.auth.password_hash.clone()
    };
    let session_token =
        session_auth::create_session_token(Some(&tenant_id), "tenant", &tenant_name, &secret, 24);

    tracing::info!("Onboard created tenant {} with default agents", tenant_name);

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "tenant_id": tenant_id,
            "tenant_name": tenant_name,
            "session_token": session_token,
        })),
    )
}

/// Find an agent.toml template by searching existing tenant workspaces.
fn find_agent_template(state: &AppState, agent_name: &str) -> Option<AgentManifest> {
    let workspaces_dir = state.kernel.config.effective_workspaces_dir();
    let Ok(entries) = std::fs::read_dir(&workspaces_dir) else {
        return None;
    };

    for entry in entries.flatten() {
        let toml_path = entry.path().join(agent_name).join("agent.toml");
        if !toml_path.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&toml_path) {
            if let Ok(manifest) = toml::from_str::<AgentManifest>(&content) {
                return Some(manifest);
            }
        }
    }
    None
}
