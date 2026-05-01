//! Route handlers for the OpenCarrier API.

pub mod agents;
pub mod auth;
pub mod bindings;
pub mod bots;
pub mod brain;
pub mod clones;
pub mod common;
pub mod comms;
pub mod config;
pub mod cron;
pub mod files;
pub mod hub;
pub mod kv;
pub mod messaging;
pub mod observability;
pub mod plugin_toml;
pub mod plugins;
pub mod providers;
pub mod sessions;
pub mod state;
pub mod tenants;
pub mod tools_skills;
pub mod webhooks;
pub mod weixin;

pub use common::*;
pub use messaging::{inject_attachments_into_session, resolve_attachments};
pub use state::AppState;

// Re-export handlers referenced by tests and external crates
pub use agents::{get_agent, kill_agent, list_agents, spawn_agent};
pub use config::get_config;
pub use messaging::send_message;
pub use observability::{health, prometheus_metrics, usage_stats};
pub use sessions::{create_agent_session, get_agent_session, list_agent_sessions, reset_session};
pub use tools_skills::list_tools;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// System-level handlers (not tied to a specific domain module)
// ---------------------------------------------------------------------------

/// GET /api/status — Kernel status.
pub async fn status(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents_owned = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|e| can_access(&ctx, e.tenant_id.as_str()))
            .collect()
    };
    let agents: Vec<serde_json::Value> = agents_owned
        .into_iter()
        .map(|e| {
            let (modality, model) = state.kernel.resolve_model_label(&e.manifest.model.modality);
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "model_provider": modality,
                "model_name": model,
                "profile": e.manifest.profile,
            })
        })
        .collect();

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();
    let (default_modality, default_model) = state.kernel.resolve_model_label("chat");

    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "default_modality": default_modality,
        "default_model": default_model,
        "uptime_seconds": uptime,
        "api_listen": state.kernel.config.api_listen,
        "home_dir": state.kernel.config.home_dir.display().to_string(),
        "log_level": state.kernel.config.log_level,
        "agents": agents,
    }))
}

/// POST /api/shutdown — Graceful shutdown.
pub async fn shutdown(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Admin only"})),
        )
            .into_response();
    }
    tracing::info!("Shutdown requested via API");
    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"})).into_response()
}

/// GET /api/version — Build & version info.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "opencarrier",
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Reset session (clear history)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ];

    if let Ok(registry) = state.kernel.plugins.skill_registry.read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// GET /api/plugins — list loaded plugin tool status.
pub async fn plugins_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
