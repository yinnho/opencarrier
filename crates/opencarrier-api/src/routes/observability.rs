//! Health, metrics, audit, logs, and usage endpoints.

use crate::routes::common::*;
use crate::routes::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;
/// GET /api/health — Minimal liveness probe (public, no auth required).
/// Returns only status and version to prevent information leakage.
/// Use GET /api/health/detail for full diagnostics (requires auth).
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Run the database check on a blocking thread so we never hold the
    // std::sync::Mutex<Connection> on a tokio worker thread.  This prevents
    // the health probe from starving the async runtime when the agent loop
    // is holding the database lock for session saves.
    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || {
        let shared_id = opencarrier_types::agent::AgentId(uuid::Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]));
        memory.structured_get(shared_id, "__health_check__").is_ok()
    })
    .await
    .unwrap_or(false);

    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
/// GET /api/health/detail — Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.runtime.supervisor.health();

    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || {
        let shared_id = opencarrier_types::agent::AgentId(uuid::Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]));
        memory.structured_get(shared_id, "__health_check__").is_ok()
    })
    .await
    .unwrap_or(false);

    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "config_warnings": config_warnings,
    }))
}
// ---------------------------------------------------------------------------
// Prometheus metrics endpoint
// ---------------------------------------------------------------------------

/// GET /api/metrics — Prometheus text-format metrics.
///
/// Returns counters and gauges for monitoring OpenCarrier in production:
/// - `opencarrier_agents_active` — number of active agents
/// - `opencarrier_uptime_seconds` — seconds since daemon started
/// - `opencarrier_tokens_total` — total tokens consumed (per agent)
/// - `opencarrier_tool_calls_total` — total tool calls (per agent)
/// - `opencarrier_panics_total` — supervisor panic count
/// - `opencarrier_restarts_total` — supervisor restart count
pub async fn prometheus_metrics(
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
    let mut out = String::with_capacity(2048);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP opencarrier_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE opencarrier_uptime_seconds gauge\n");
    out.push_str(&format!("opencarrier_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, opencarrier_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP opencarrier_agents_active Number of active agents.\n");
    out.push_str("# TYPE opencarrier_agents_active gauge\n");
    out.push_str(&format!("opencarrier_agents_active {active}\n"));
    out.push_str("# HELP opencarrier_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE opencarrier_agents_total gauge\n");
    out.push_str(&format!("opencarrier_agents_total {}\n\n", agents.len()));

    // Per-agent token and tool usage
    out.push_str(
        "# HELP opencarrier_tokens_total Total tokens consumed (rolling hourly window).\n",
    );
    out.push_str("# TYPE opencarrier_tokens_total gauge\n");
    out.push_str("# HELP opencarrier_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE opencarrier_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let modality = &agent.manifest.model.modality;
        let model = &agent.manifest.model.modality;
        if let Some((tokens, tools)) = state.kernel.runtime.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "opencarrier_tokens_total{{agent=\"{name}\",modality=\"{modality}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "opencarrier_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.runtime.supervisor.health();
    out.push_str("# HELP opencarrier_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE opencarrier_panics_total counter\n");
    out.push_str(&format!(
        "opencarrier_panics_total {}\n",
        health.panic_count
    ));
    out.push_str("# HELP opencarrier_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE opencarrier_restarts_total counter\n");
    out.push_str(&format!(
        "opencarrier_restarts_total {}\n\n",
        health.restart_count
    ));

    // Version info
    out.push_str("# HELP opencarrier_info OpenCarrier version and build info.\n");
    out.push_str("# TYPE opencarrier_info gauge\n");
    out.push_str(&format!(
        "opencarrier_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
        .into_response()
}
// ---------------------------------------------------------------------------
// Audit endpoints
// ---------------------------------------------------------------------------

/// GET /api/audit/recent — Get recent audit log entries.
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Admin only"})),
        )
            .into_response();
    }
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000); // Cap at 1000

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    }))
    .into_response()
}
/// GET /api/audit/verify — Verify the audit chain integrity.
pub async fn audit_verify(
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
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
                .into_response()
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
                .into_response()
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        }))
        .into_response(),
    }
}
/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///   - `token`  — auth token (for EventSource clients that cannot set headers)
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second and sends only new entries
/// (tracked by sequence number). On first connect, existing entries are sent
/// as a backfill so the client has immediate context.
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Admin only"})),
        )
            .into_response();
    }
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = state.kernel.audit_log.recent(200);

            for entry in &entries {
                // On first poll, send all existing entries as backfill.
                // After that, only send entries newer than last_seq.
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}
// ---------------------------------------------------------------------------
// Usage endpoint
// ---------------------------------------------------------------------------

/// GET /api/usage — Get per-agent usage statistics.
pub async fn usage_stats(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents_filtered: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|e| can_access(&ctx, &e.tenant_id))
            .collect()
    };
    let agents: Vec<serde_json::Value> = agents_filtered
        .iter()
        .map(|e| {
            let (tokens, tool_calls) = state
                .kernel
                .runtime
                .scheduler
                .get_usage(e.id)
                .unwrap_or((0, 0));
            serde_json::json!({
                "agent_id": e.id.to_string(),
                "name": e.name,
                "total_tokens": tokens,
                "tool_calls": tool_calls,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents}))
}
// ---------------------------------------------------------------------------
// Usage summary endpoints
// ---------------------------------------------------------------------------

/// GET /api/usage/summary — Get overall usage summary from UsageStore.
pub async fn usage_summary(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() {
        None
    } else {
        ctx.tenant_id.clone()
    };
    match state
        .kernel
        .memory
        .usage()
        .query_summary(None, tid.as_deref())
    {
        Ok(s) => Json(serde_json::json!({
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "call_count": s.call_count,
            "total_tool_calls": s.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}
/// GET /api/usage/by-model — Get usage grouped by model.
pub async fn usage_by_model(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() {
        None
    } else {
        ctx.tenant_id.clone()
    };
    match state.kernel.memory.usage().query_by_model(tid.as_deref()) {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_input_tokens": m.total_input_tokens,
                        "total_output_tokens": m.total_output_tokens,
                        "call_count": m.call_count,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}
/// GET /api/usage/daily — Get daily usage breakdown for the last 7 days.
pub async fn usage_daily(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let tid = if ctx.is_admin() {
        None
    } else {
        ctx.tenant_id.clone()
    };
    let days = state
        .kernel
        .memory
        .usage()
        .query_daily_breakdown(7, tid.as_deref());
    let first_event = state.kernel.memory.usage().query_first_event_date();

    let days_list = match days {
        Ok(d) => d
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "first_event_date": first_event.unwrap_or(None),
    }))
}
// ---------------------------------------------------------------------------
// Budget endpoint
// ---------------------------------------------------------------------------

/// GET /api/budget — Get budget configuration and monthly usage status.
pub async fn budget_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = state.kernel.metering.get_budget_status();
    Json(serde_json::json!({
        "used_tokens": status.used_tokens,
        "limit_tokens": status.limit_tokens,
        "percent": status.percent,
        "fired_thresholds": status.fired_thresholds,
        "thresholds": status.thresholds,
        "alert_channel": status.alert_channel,
        "alert_recipient": status.alert_recipient,
        "budget_exceeded": status.limit_tokens > 0 && status.used_tokens > status.limit_tokens,
    }))
}

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------

/// GET /api/security — Security feature status for the dashboard.
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = if state.kernel.config.api_key.is_empty() {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit_log.len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.is_empty()
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/audit/recent", routing::get(audit_recent))
        .route("/api/audit/verify", routing::get(audit_verify))
        .route("/api/health", routing::get(health))
        .route("/api/health/detail", routing::get(health_detail))
        .route("/api/logs/stream", routing::get(logs_stream))
        .route("/api/metrics", routing::get(prometheus_metrics))
        .route("/api/security", routing::get(security_status))
        .route("/api/usage", routing::get(usage_stats))
        .route("/api/usage/by-model", routing::get(usage_by_model))
        .route("/api/usage/daily", routing::get(usage_daily))
        .route("/api/usage/summary", routing::get(usage_summary))
        .route("/api/budget", routing::get(budget_status))
}
