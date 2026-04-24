//! Agent communication (comms) endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;
// ---------------------------------------------------------------------------
// Agent Communication (Comms) endpoints
// ---------------------------------------------------------------------------

/// GET /api/comms/topology — Build agent topology graph from registry.
pub async fn comms_topology(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> impl IntoResponse {
    use opencarrier_types::comms::{EdgeKind, TopoEdge, TopoNode, Topology};

    let ctx = get_tenant_ctx(&extensions);
    let all_agents = state.kernel.registry.list();
    let agents: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|a| can_access(&ctx, a.tenant_id.as_deref()))
            .collect()
    };

    let nodes: Vec<TopoNode> = agents
        .iter()
        .map(|e| TopoNode {
            id: e.id.to_string(),
            name: e.name.clone(),
            state: format!("{:?}", e.state),
            model: e.manifest.model.modality.clone(),
        })
        .collect();

    let mut edges: Vec<TopoEdge> = Vec::new();

    // Parent-child edges from registry
    for agent in &agents {
        for child_id in &agent.children {
            edges.push(TopoEdge {
                from: agent.id.to_string(),
                to: child_id.to_string(),
                kind: EdgeKind::ParentChild,
            });
        }
    }

    // Peer message edges from event bus history
    let events = state.kernel.coordination.event_bus.history(500).await;
    let mut peer_pairs = std::collections::HashSet::new();
    for event in &events {
        if let opencarrier_types::event::EventPayload::Message(_) = &event.payload {
            if let opencarrier_types::event::EventTarget::Agent(target_id) = &event.target {
                let from = event.source.to_string();
                let to = target_id.to_string();
                // Deduplicate: only one edge per pair, skip self-loops
                if from != to {
                    let key = if from < to {
                        (from.clone(), to.clone())
                    } else {
                        (to.clone(), from.clone())
                    };
                    if peer_pairs.insert(key) {
                        edges.push(TopoEdge {
                            from,
                            to,
                            kind: EdgeKind::Peer,
                        });
                    }
                }
            }
        }
    }

    Json(serde_json::to_value(Topology { nodes, edges }).unwrap_or_default())
}

/// Filter a kernel event into a CommsEvent, if it represents inter-agent communication.
fn filter_to_comms_event(
    event: &opencarrier_types::event::Event,
    agents: &[opencarrier_types::agent::AgentEntry],
) -> Option<opencarrier_types::comms::CommsEvent> {
    use opencarrier_types::comms::{CommsEvent, CommsEventKind};
    use opencarrier_types::event::{EventPayload, EventTarget, LifecycleEvent};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    match &event.payload {
        EventPayload::Message(msg) => {
            let target_id = match &event.target {
                EventTarget::Agent(id) => id.to_string(),
                _ => String::new(),
            };
            Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentMessage,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: target_id.clone(),
                target_name: resolve_name(&target_id),
                detail: opencarrier_types::truncate_str(&msg.content, 200).to_string(),
            })
        }
        EventPayload::Lifecycle(lifecycle) => match lifecycle {
            LifecycleEvent::Spawned { agent_id, name } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentSpawned,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: name.clone(),
                detail: format!("Agent '{}' spawned", name),
            }),
            LifecycleEvent::Terminated { agent_id, reason } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentTerminated,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: resolve_name(&agent_id.to_string()),
                detail: format!("Terminated: {}", reason),
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Convert an audit entry into a CommsEvent if it represents inter-agent activity.
fn audit_to_comms_event(
    entry: &opencarrier_runtime::audit::AuditEntry,
    agents: &[opencarrier_types::agent::AgentEntry],
) -> Option<opencarrier_types::comms::CommsEvent> {
    use opencarrier_types::comms::{CommsEvent, CommsEventKind};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if id.is_empty() || id == "system" {
                    "system".to_string()
                } else {
                    opencarrier_types::truncate_str(id, 12).to_string()
                }
            })
    };

    let action_str = format!("{:?}", entry.action);
    let (kind, detail, target_label) = match action_str.as_str() {
        "AgentMessage" => {
            // Format detail: "tokens_in=X, tokens_out=Y" → readable summary
            let detail = if entry.detail.starts_with("tokens_in=") {
                let parts: Vec<&str> = entry.detail.split(", ").collect();
                let in_tok = parts
                    .first()
                    .and_then(|p| p.strip_prefix("tokens_in="))
                    .unwrap_or("?");
                let out_tok = parts
                    .get(1)
                    .and_then(|p| p.strip_prefix("tokens_out="))
                    .unwrap_or("?");
                if entry.outcome == "ok" {
                    format!("{} in / {} out tokens", in_tok, out_tok)
                } else {
                    format!(
                        "{} in / {} out — {}",
                        in_tok,
                        out_tok,
                        opencarrier_types::truncate_str(&entry.outcome, 80)
                    )
                }
            } else if entry.outcome != "ok" {
                format!(
                    "{} — {}",
                    opencarrier_types::truncate_str(&entry.detail, 80),
                    opencarrier_types::truncate_str(&entry.outcome, 80)
                )
            } else {
                opencarrier_types::truncate_str(&entry.detail, 200).to_string()
            };
            (CommsEventKind::AgentMessage, detail, "user")
        }
        "AgentSpawn" => (
            CommsEventKind::AgentSpawned,
            format!(
                "Agent spawned: {}",
                opencarrier_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        "AgentKill" => (
            CommsEventKind::AgentTerminated,
            format!(
                "Agent killed: {}",
                opencarrier_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        _ => return None,
    };

    Some(CommsEvent {
        id: format!("audit-{}", entry.seq),
        timestamp: entry.timestamp.clone(),
        kind,
        source_id: entry.agent_id.clone(),
        source_name: resolve_name(&entry.agent_id),
        target_id: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        target_name: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        detail,
    })
}
/// GET /api/comms/events — Return recent inter-agent communication events.
///
/// Sources from both the event bus (for lifecycle events with full context)
/// and the audit log (for message/spawn/kill events that are always captured).
pub async fn comms_events(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);

    let all_agents = state.kernel.registry.list();
    let agents: Vec<_> = if ctx.is_admin() {
        all_agents
    } else {
        all_agents
            .into_iter()
            .filter(|a| can_access(&ctx, a.tenant_id.as_deref()))
            .collect()
    };

    // Primary source: event bus (has full source/target context)
    let bus_events = state.kernel.coordination.event_bus.history(500).await;
    let mut comms_events: Vec<opencarrier_types::comms::CommsEvent> = bus_events
        .iter()
        .filter_map(|e| filter_to_comms_event(e, &agents))
        .collect();

    // Secondary source: audit log (always populated, wider coverage)
    let audit_entries = state.kernel.audit_log.recent(500);
    let seen_ids: std::collections::HashSet<String> =
        comms_events.iter().map(|e| e.id.clone()).collect();

    for entry in audit_entries.iter().rev() {
        if let Some(ev) = audit_to_comms_event(entry, &agents) {
            if !seen_ids.contains(&ev.id) {
                comms_events.push(ev);
            }
        }
    }

    // Sort by timestamp descending (newest first)
    comms_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    comms_events.truncate(limit);

    Json(comms_events)
}
/// GET /api/comms/events/stream — SSE stream of inter-agent communication events.
///
/// Polls the audit log every 500ms for new inter-agent events.
pub async fn comms_events_stream(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
) -> axum::response::Response {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, "Admin only").into_response();
    }
    use axum::response::sse::{Event, KeepAlive, Sse};

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = {
            let entries = state.kernel.audit_log.recent(1);
            entries.last().map(|e| e.seq).unwrap_or(0)
        };

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let agents = state.kernel.registry.list();
            let entries = state.kernel.audit_log.recent(50);

            for entry in &entries {
                if entry.seq <= last_seq {
                    continue;
                }
                if let Some(comms_event) = audit_to_comms_event(entry, &agents) {
                    let data = serde_json::to_string(&comms_event).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        return; // Client disconnected
                    }
                }
            }

            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
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
/// POST /api/comms/send — Send a message from one agent to another.
pub async fn comms_send(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<opencarrier_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);

    // Validate from agent exists and tenant can access it
    let from_id = match parse_agent_id(&req.from_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Some(from_entry) = state.kernel.registry.get(from_id) {
        if !can_access(&ctx, from_entry.tenant_id.as_deref()) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied to source agent"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Source agent not found"})));
    }

    // Validate to agent exists and tenant can access it
    let to_id = match parse_agent_id(&req.to_agent_id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Some(to_entry) = state.kernel.registry.get(to_id) {
        if !can_access(&ctx, to_entry.tenant_id.as_deref()) {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Access denied to target agent"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Target agent not found"})));
    }

    // SECURITY: Limit message size
    if req.message.len() > 64 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    match state.kernel.send_message(to_id, &req.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "response": result.response,
                "input_tokens": result.total_usage.input_tokens,
                "output_tokens": result.total_usage.output_tokens,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
        ),
    }
}
/// POST /api/comms/task — Post a task to the agent task queue.
pub async fn comms_task(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Json(req): Json<opencarrier_types::comms::CommsTaskRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    if !ctx.is_admin() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "Admin only"})));
    }
    if req.title.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Title is required"})),
        );
    }

    match state
        .kernel
        .memory
        .task_post(
            &req.title,
            &req.description,
            req.assigned_to.as_deref(),
            Some("ui-user"),
        )
        .await
    {
        Ok(task_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "ok": true,
                "task_id": task_id,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to post task: {e}")})),
        ),
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/comms/events", routing::get(comms_events))
        .route("/api/comms/events/stream", routing::get(comms_events_stream))
        .route("/api/comms/send", routing::post(comms_send))
        .route("/api/comms/task", routing::post(comms_task))
        .route("/api/comms/topology", routing::get(comms_topology))
}
