//! Cron job management endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use opencarrier_runtime::kernel_handle::KernelHandle;
use opencarrier_types::agent::AgentId;
use std::collections::HashMap;
use std::sync::Arc;
// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                let jobs = state.kernel.cron_scheduler.list_jobs(aid);
                if ctx.is_admin() {
                    jobs
                } else {
                    jobs.into_iter().filter(|j| can_access(&ctx, j.tenant_id.as_deref())).collect()
                }
            }
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid agent_id"})),
                );
            }
        }
    } else if ctx.is_admin() {
        state.kernel.cron_scheduler.list_all_jobs()
    } else {
        state.kernel.cron_scheduler.list_all_jobs_by_tenant(ctx.tenant_id.as_deref().unwrap_or(""))
    };
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}
/// POST /api/cron/jobs — Create a new cron job.
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    // Verify tenant owns the target agent
    if let Ok(aid) = agent_id.parse::<AgentId>() {
        if let Some(entry) = state.kernel.registry.get(aid) {
            if !can_access(&ctx, entry.tenant_id.as_deref()) {
                return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
            }
        }
    }
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"result": result})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
}
/// DELETE /api/cron/jobs/{id} — Delete a cron job.
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.remove_job(job_id) {
                Ok(_) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "deleted"})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}
/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.set_enabled(job_id, enabled) {
                Ok(()) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}
/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = opencarrier_types::scheduler::CronJobId(uuid);
            // Tenant ownership check
            if let Some(job) = state.kernel.cron_scheduler.get_job(job_id) {
                if !can_access(&ctx, job.tenant_id.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied"})));
                }
            }
            match state.kernel.cron_scheduler.get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Job not found"})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/cron/jobs", routing::post(create_cron_job).get(list_cron_jobs))
        .route("/api/cron/jobs/{id}", routing::delete(delete_cron_job))
        .route("/api/cron/jobs/{id}/enable", routing::put(toggle_cron_job))
        .route("/api/cron/jobs/{id}/status", routing::get(cron_job_status))
}
