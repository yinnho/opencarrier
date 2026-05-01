//! Authentication endpoints.

use crate::routes::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
// ── Dashboard Authentication (username/password sessions) ──

/// POST /api/auth/login — Authenticate with username/password, returns session token.
///
/// First checks the tenants table (multi-tenant login), then falls back to
/// config.toml credentials (legacy admin login).
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::Response;

    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Auth not enabled"}).to_string(),
            ))
            .unwrap();
    }

    let username = req.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = req.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    // Step 1: Try tenants table (multi-tenant login)
    let tenant_store = state.kernel.memory.tenant();
    if let Ok(Some(tenant)) = tenant_store.get_tenant_by_name(username) {
        if !tenant.enabled {
            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::AuthAttempt,
                "dashboard login failed (tenant disabled)",
                format!("username: {username}"),
            );
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "Invalid credentials"}).to_string(),
                ))
                .unwrap();
        }
        if crate::session_auth::verify_password(password, &tenant.password_hash) {
            let token = crate::session_auth::create_session_token(
                Some(&tenant.id),
                tenant.role.as_str(),
                &tenant.name,
                &secret,
                auth_cfg.session_ttl_hours,
            );
            let ttl_secs = auth_cfg.session_ttl_hours * 3600;
            let cookie = format!(
                "opencarrier_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}"
            );

            state.kernel.audit_log.record(
                "system",
                opencarrier_runtime::audit::AuditAction::AuthAttempt,
                "dashboard login success (tenant)",
                format!("username: {username}, role: {}", tenant.role.as_str()),
            );

            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("set-cookie", &cookie)
                .body(Body::from(
                    serde_json::json!({
                        "status": "ok",
                        "token": token,
                        "username": username,
                        "role": tenant.role.as_str(),
                        "tenant_id": tenant.id,
                    })
                    .to_string(),
                ))
                .unwrap();
        }
    }

    // Step 2: Fallback to config.toml credentials (legacy admin login)
    // Constant-time username comparison to prevent timing attacks
    let username_ok = {
        use subtle::ConstantTimeEq;
        let stored = auth_cfg.username.as_bytes();
        let provided = username.as_bytes();
        if stored.len() != provided.len() {
            false
        } else {
            bool::from(stored.ct_eq(provided))
        }
    };

    if !username_ok || !crate::session_auth::verify_password(password, &auth_cfg.password_hash) {
        // Audit log the failed attempt
        state.kernel.audit_log.record(
            "system",
            opencarrier_runtime::audit::AuditAction::AuthAttempt,
            "dashboard login failed",
            format!("username: {username}"),
        );
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Invalid credentials"}).to_string(),
            ))
            .unwrap();
    }

    // Legacy admin login — ensure a tenant record exists for this admin,
    // then issue token with the admin's tenant_id so workspace paths are scoped.
    let admin_tid = {
        let tenant_store = state.kernel.memory.tenant();
        match tenant_store.get_tenant_by_name(username) {
            Ok(Some(t)) => t.id,
            _ => {
                // Auto-create tenant record for legacy admin
                let id = uuid::Uuid::new_v4().to_string();
                let hash = auth_cfg.password_hash.clone();
                let entry = opencarrier_types::tenant::TenantEntry {
                    id: id.clone(),
                    name: username.to_string(),
                    password_hash: hash,
                    role: opencarrier_types::tenant::TenantRole::Admin,
                    enabled: true,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                };
                if let Err(e) = tenant_store.create_tenant(&entry) {
                    tracing::warn!("Failed to auto-create admin tenant: {e}");
                }
                id
            }
        }
    };
    let token = crate::session_auth::create_session_token(
        Some(&admin_tid),
        "admin",
        username,
        &secret,
        auth_cfg.session_ttl_hours,
    );
    let ttl_secs = auth_cfg.session_ttl_hours * 3600;
    let cookie = format!(
        "opencarrier_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}"
    );

    state.kernel.audit_log.record(
        "system",
        opencarrier_runtime::audit::AuditAction::AuthAttempt,
        "dashboard login success (legacy admin)",
        format!("username: {username}"),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", &cookie)
        .body(Body::from(
            serde_json::json!({
                "status": "ok",
                "token": token,
                "username": username,
                "role": "admin",
                "tenant_id": admin_tid,
            })
            .to_string(),
        ))
        .unwrap()
}
/// POST /api/auth/logout — Clear the session cookie.
pub async fn auth_logout() -> impl IntoResponse {
    let cookie = "opencarrier_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [("content-type", "application/json"), ("set-cookie", cookie)],
        serde_json::json!({"status": "ok"}).to_string(),
    )
}
/// GET /api/auth/check — Check current authentication state.
pub async fn auth_check(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        return Json(serde_json::json!({
            "authenticated": true,
            "mode": "none",
        }));
    }

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    // Check session cookie
    let session_user = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("opencarrier_session=")
                    .map(|v| v.to_string())
            })
        })
        .and_then(|token| crate::session_auth::verify_session_token(&token, &secret));

    if let Some(info) = session_user {
        Json(serde_json::json!({
            "authenticated": true,
            "mode": "session",
            "username": info.username,
            "role": info.role,
            "tenant_id": info.tenant_id,
        }))
    } else {
        Json(serde_json::json!({
            "authenticated": false,
            "mode": "session",
        }))
    }
}

/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new()
        .route("/api/auth/login", routing::post(auth_login))
        .route("/api/auth/check", routing::get(auth_check))
        .route("/api/auth/logout", routing::post(auth_logout))
}
