//! Invite tracking API — stats for share-page referral analytics.

use crate::routes::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/invites/stats", routing::get(invite_stats))
        .route("/api/invites/leaderboard", routing::get(leaderboard))
}

/// GET /api/invites/stats — query invite stats for the current inviter.
/// Accepts ?inviter_fp=xxx or falls back to X-Fingerprint header.
pub async fn invite_stats(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let inviter_fp = query
        .get("inviter_fp")
        .cloned()
        .or_else(|| {
            headers
                .get("x-fingerprint")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    if inviter_fp.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing inviter_fp"})),
        );
    }

    match state.kernel.memory.invites().query_stats(&inviter_fp) {
        Ok(stats) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({
                "inviter_fp": inviter_fp,
                "total_invites": stats.total_invites,
                "converted": stats.converted,
                "pending": stats.pending,
            })),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Database error: {e}")})),
        ),
    }
}

/// GET /api/invites/leaderboard — top inviters by conversion count.
pub async fn leaderboard(
    State(state): State<Arc<AppState>>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let limit: usize = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    match state.kernel.memory.invites().top_inviters(limit) {
        Ok(rows) => {
            let entries: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|(fp, total, converted)| {
                    serde_json::json!({
                        "inviter_fp": fp,
                        "total_invites": total,
                        "converted": converted,
                    })
                })
                .collect();
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({ "leaderboard": entries })),
            )
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Database error: {e}")})),
        ),
    }
}
