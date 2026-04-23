//! OpenCarrier daemon server — boots the kernel and serves the HTTP API.

use crate::middleware;
use crate::rate_limiter;
use crate::routes::{self, AppState};
use crate::webchat;
use crate::ws;
use axum::Router;
use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_runtime::plugin::PluginManager;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Daemon info written to `~/.opencarrier/daemon.json` so the CLI can find us.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub listen_addr: String,
    pub started_at: String,
    pub version: String,
    pub platform: String,
}

/// Build the full API router with all routes, middleware, and state.
///
/// This is extracted from `run_daemon()` so that embedders (e.g. opencarrier-desktop)
/// can create the router without starting the full daemon lifecycle.
///
/// Returns `(router, shared_state)`.
pub async fn build_router(
    kernel: Arc<OpenCarrierKernel>,
    listen_addr: SocketAddr,
    plugin_manager: Option<PluginManager>,
) -> (Router<()>, Arc<AppState>) {
    let state = Arc::new(AppState {
        kernel: kernel.clone(),
        started_at: Instant::now(),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        provider_probe_cache: opencarrier_runtime::provider_health::ProbeCache::new(),
        plugin_manager: plugin_manager.map(|pm| Arc::new(tokio::sync::Mutex::new(pm))),
    });

    // CORS: allow localhost origins by default. If API key is set, the API
    // is protected anyway. For development, permissive CORS is convenient.
    let cors = if state.kernel.config.api_key.trim().is_empty() {
        // No auth → restrict CORS to localhost origins (include both 127.0.0.1 and localhost)
        let port = listen_addr.port();
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            format!("http://localhost:{port}").parse().unwrap(),
        ];
        // Also allow common dev ports
        for p in [3000u16, 8080] {
            if p != port {
                if let Ok(v) = format!("http://127.0.0.1:{p}").parse() {
                    origins.push(v);
                }
                if let Ok(v) = format!("http://localhost:{p}").parse() {
                    origins.push(v);
                }
            }
        }
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        // Auth enabled → restrict CORS to localhost + configured origins.
        // SECURITY: CorsLayer::permissive() is dangerous — any website could
        // make cross-origin requests. Restrict to known origins instead.
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            "http://localhost:4200".parse().unwrap(),
            "http://127.0.0.1:4200".parse().unwrap(),
            "http://localhost:8080".parse().unwrap(),
            "http://127.0.0.1:8080".parse().unwrap(),
        ];
        // Add the actual listen address variants
        if listen_addr.port() != 4200 && listen_addr.port() != 8080 {
            if let Ok(v) = format!("http://localhost:{}", listen_addr.port()).parse() {
                origins.push(v);
            }
            if let Ok(v) = format!("http://127.0.0.1:{}", listen_addr.port()).parse() {
                origins.push(v);
            }
        }
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    };

    // Trim whitespace so `api_key = ""` or `api_key = "  "` both disable auth.
    let api_key = state.kernel.config.api_key.trim().to_string();
    let auth_state = crate::middleware::AuthState {
        api_key: api_key.clone(),
        auth_enabled: state.kernel.config.auth.enabled,
        session_secret: if !api_key.is_empty() {
            api_key.clone()
        } else if state.kernel.config.auth.enabled {
            state.kernel.config.auth.password_hash.clone()
        } else {
            String::new()
        },
    };
    let gcra_limiter = rate_limiter::create_rate_limiter();

    let app = Router::new()
        .route("/", axum::routing::get(webchat::webchat_page))
        .route("/logo.png", axum::routing::get(webchat::logo_png))
        .route("/favicon.ico", axum::routing::get(webchat::favicon_ico))
        .route("/manifest.json", axum::routing::get(webchat::manifest_json))
        .route("/sw.js", axum::routing::get(webchat::sw_js))
        .route(
            "/api/metrics",
            axum::routing::get(routes::prometheus_metrics),
        )
        .route("/api/health", axum::routing::get(routes::health))
        .route(
            "/api/health/detail",
            axum::routing::get(routes::health_detail),
        )
        .route("/api/status", axum::routing::get(routes::status))
        // Provider API Key management
        .route(
            "/api/providers/keys",
            axum::routing::get(routes::list_provider_keys),
        )
        .route(
            "/api/providers/{name}/key",
            axum::routing::post(routes::set_provider_key)
                .delete(routes::delete_provider_key),
        )
        .route("/api/brain", axum::routing::get(routes::brain_info))
        .route("/api/brain/status", axum::routing::get(routes::brain_status))
        .route(
            "/api/brain/modalities/{name}",
            axum::routing::get(routes::brain_modality_detail),
        )
        // Brain config management
        .route(
            "/api/brain/providers/{name}",
            axum::routing::put(routes::set_brain_provider)
                         .delete(routes::delete_brain_provider),
        )
        .route(
            "/api/brain/endpoints/{name}",
            axum::routing::put(routes::set_brain_endpoint)
                         .delete(routes::delete_brain_endpoint),
        )
        .route(
            "/api/brain/modalities/{name}",
            axum::routing::put(routes::set_brain_modality)
                         .delete(routes::delete_brain_modality),
        )
        .route(
            "/api/brain/default-modality",
            axum::routing::put(routes::set_brain_default_modality),
        )
        .route(
            "/api/brain/reload",
            axum::routing::post(routes::reload_brain),
        )
        .route(
            "/api/brain/config",
            axum::routing::get(routes::get_brain_config_raw)
                .put(routes::put_brain_config_raw),
        )
        .route("/api/version", axum::routing::get(routes::version))
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/{id}",
            axum::routing::get(routes::get_agent)
                .delete(routes::kill_agent)
                .patch(routes::patch_agent),
        )
        .route(
            "/api/agents/{id}/mode",
            axum::routing::put(routes::set_agent_mode),
        )
        .route("/api/profiles", axum::routing::get(routes::list_profiles))
        .route(
            "/api/agents/{id}/restart",
            axum::routing::post(routes::restart_agent),
        )
        .route(
            "/api/agents/{id}/start",
            axum::routing::post(routes::restart_agent),
        )
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/message/stream",
            axum::routing::post(routes::send_message_stream),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route(
            "/api/agents/{id}/sessions",
            axum::routing::get(routes::list_agent_sessions).post(routes::create_agent_session),
        )
        .route(
            "/api/agents/{id}/sessions/{session_id}/switch",
            axum::routing::post(routes::switch_agent_session),
        )
        .route(
            "/api/agents/{id}/session/reset",
            axum::routing::post(routes::reset_session),
        )
        .route(
            "/api/agents/{id}/history",
            axum::routing::delete(routes::clear_agent_history),
        )
        .route(
            "/api/agents/{id}/session/compact",
            axum::routing::post(routes::compact_session),
        )
        .route(
            "/api/agents/{id}/stop",
            axum::routing::post(routes::stop_agent),
        )
        .route(
            "/api/agents/{id}/model",
            axum::routing::put(routes::set_model),
        )
        .route(
            "/api/agents/{id}/tools",
            axum::routing::get(routes::get_agent_tools).put(routes::set_agent_tools),
        )
        .route(
            "/api/agents/{id}/skills",
            axum::routing::get(routes::get_agent_skills).put(routes::set_agent_skills),
        )
        .route(
            "/api/agents/{id}/mcp_servers",
            axum::routing::get(routes::get_agent_mcp_servers).put(routes::set_agent_mcp_servers),
        )
        .route(
            "/api/agents/{id}/identity",
            axum::routing::patch(routes::update_agent_identity),
        )
        .route(
            "/api/agents/{id}/config",
            axum::routing::patch(routes::patch_agent_config),
        )
        .route(
            "/api/agents/{id}/clone",
            axum::routing::post(routes::clone_agent),
        )
        .route(
            "/api/agents/{id}/files",
            axum::routing::get(routes::list_agent_files),
        )
        .route(
            "/api/agents/{id}/files/{filename}",
            axum::routing::get(routes::get_agent_file).put(routes::set_agent_file),
        )
        .route(
            "/api/agents/{id}/upload",
            axum::routing::post(routes::upload_file),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        // Upload serving
        .route(
            "/api/uploads/{file_id}",
            axum::routing::get(routes::serve_upload),
        )
        // Template endpoints
        .route("/api/templates", axum::routing::get(routes::list_templates))
        .route(
            "/api/templates/{name}",
            axum::routing::get(routes::get_template),
        )
        // Hub marketplace endpoints
        .route("/api/hub/templates", axum::routing::get(routes::list_hub_templates))
        .route(
            "/api/hub/templates/{name}/install",
            axum::routing::post(routes::install_hub_template),
        )
        // Clone (.agx) endpoints
        .route("/api/clones", axum::routing::get(routes::list_clones))
        .route(
            "/api/clones/install",
            axum::routing::post(routes::install_clone),
        )
        .route(
            "/api/clones/{name}/start",
            axum::routing::post(routes::start_clone),
        )
        .route(
            "/api/clones/{name}/stop",
            axum::routing::post(routes::stop_clone),
        )
        .route(
            "/api/clones/{name}",
            axum::routing::delete(routes::uninstall_clone),
        )
        // Clone lifecycle endpoints
        .route(
            "/api/clones/{name}/compile",
            axum::routing::post(routes::clone_compile),
        )
        .route(
            "/api/clones/{name}/health",
            axum::routing::get(routes::clone_health),
        )
        .route(
            "/api/clones/{name}/rollback",
            axum::routing::post(routes::clone_rollback),
        )
        .route(
            "/api/clones/{name}/verify",
            axum::routing::post(routes::clone_verify),
        )
        .route(
            "/api/clones/{name}/feedback/push",
            axum::routing::post(routes::clone_feedback_push),
        )
        .route(
            "/api/clones/{name}/evaluate",
            axum::routing::get(routes::clone_evaluate),
        )
        // Memory endpoints
        .route(
            "/api/memory/agents/{id}/kv",
            axum::routing::get(routes::get_agent_kv),
        )
        .route(
            "/api/memory/agents/{id}/kv/{key}",
            axum::routing::get(routes::get_agent_kv_key)
                .put(routes::set_agent_kv_key)
                .delete(routes::delete_agent_kv_key),
        )
        // Skills endpoints
        .route("/api/skills", axum::routing::get(routes::list_skills))
        // MCP server endpoints
        .route(
            "/api/mcp/servers",
            axum::routing::get(routes::list_mcp_servers),
        )
        // Audit endpoints
        .route(
            "/api/audit/recent",
            axum::routing::get(routes::audit_recent),
        )
        .route(
            "/api/audit/verify",
            axum::routing::get(routes::audit_verify),
        )
        // Live log streaming (SSE)
        .route("/api/logs/stream", axum::routing::get(routes::logs_stream))
        // Agent communication (Comms) endpoints
        .route(
            "/api/comms/topology",
            axum::routing::get(routes::comms_topology),
        )
        .route(
            "/api/comms/events",
            axum::routing::get(routes::comms_events),
        )
        .route(
            "/api/comms/events/stream",
            axum::routing::get(routes::comms_events_stream),
        )
        .route("/api/comms/send", axum::routing::post(routes::comms_send))
        .route("/api/comms/task", axum::routing::post(routes::comms_task))
        .route("/api/plugins", axum::routing::get(routes::plugins_list))
        // WeChat iLink Bot — QR code binding
        .route("/api/weixin/qrcode", axum::routing::get(routes::weixin_qrcode))
        .route(
            "/api/weixin/qrcode-status",
            axum::routing::get(routes::weixin_qrcode_status),
        )
        .route("/api/weixin/status", axum::routing::get(routes::weixin_status))
        // Unified channels management
        .route(
            "/api/channels/status",
            axum::routing::get(routes::channels_status),
        )
        .route(
            "/api/channels/wecom/tenants",
            axum::routing::post(routes::wecom_add_tenant),
        )
        .route(
            "/api/channels/feishu/tenants",
            axum::routing::post(routes::feishu_add_tenant),
        );

    // Split into a second router chunk to stay within axum's type nesting limit.
    let app = app
        // Tools endpoint
        .route("/api/tools", axum::routing::get(routes::list_tools))
        // Config endpoints
        .route("/api/config", axum::routing::get(routes::get_config))
        .route(
            "/api/config/schema",
            axum::routing::get(routes::config_schema),
        )
        .route("/api/config/set", axum::routing::post(routes::config_set))
        // Usage endpoints
        .route("/api/usage", axum::routing::get(routes::usage_stats))
        .route(
            "/api/usage/summary",
            axum::routing::get(routes::usage_summary),
        )
        .route(
            "/api/usage/by-model",
            axum::routing::get(routes::usage_by_model),
        )
        .route("/api/usage/daily", axum::routing::get(routes::usage_daily))
        // Session endpoints
        .route("/api/sessions", axum::routing::get(routes::list_sessions))
        .route(
            "/api/sessions/{id}",
            axum::routing::delete(routes::delete_session),
        )
        .route(
            "/api/sessions/{id}/label",
            axum::routing::put(routes::set_session_label),
        )
        .route(
            "/api/agents/{id}/sessions/by-label/{label}",
            axum::routing::get(routes::find_session_by_label),
        )
        // Agent update
        .route(
            "/api/agents/{id}/update",
            axum::routing::put(routes::update_agent),
        )
        // Security dashboard endpoint
        .route("/api/security", axum::routing::get(routes::security_status))
        .route(
            "/api/skills/create",
            axum::routing::post(routes::create_skill),
        )
        // Cron job management endpoints
        .route(
            "/api/cron/jobs",
            axum::routing::get(routes::list_cron_jobs).post(routes::create_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}",
            axum::routing::delete(routes::delete_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}/enable",
            axum::routing::put(routes::toggle_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}/status",
            axum::routing::get(routes::cron_job_status),
        )
        // Webhook trigger endpoints (external event injection)
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        // Chat commands endpoint (dynamic slash menu)
        .route("/api/commands", axum::routing::get(routes::list_commands))
        // Config reload endpoint
        .route(
            "/api/config/reload",
            axum::routing::post(routes::config_reload),
        )
        // Agent binding routes
        .route(
            "/api/bindings",
            axum::routing::get(routes::list_bindings).post(routes::add_binding),
        )
        .route(
            "/api/bindings/{index}",
            axum::routing::delete(routes::remove_binding),
        )
        // MCP HTTP endpoint (exposes MCP protocol over HTTP)
        .route("/mcp", axum::routing::post(routes::mcp_http))
        // Dashboard authentication endpoints
        .route("/api/auth/login", axum::routing::post(routes::auth_login))
        .route("/api/auth/logout", axum::routing::post(routes::auth_logout))
        .route("/api/auth/check", axum::routing::get(routes::auth_check))
        // Tenant management endpoints (admin only)
        .route(
            "/api/tenants",
            axum::routing::get(routes::list_tenants).post(routes::create_tenant),
        )
        .route(
            "/api/tenants/{id}",
            axum::routing::get(routes::get_tenant)
                .put(routes::update_tenant)
                .delete(routes::delete_tenant),
        )
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(axum::middleware::from_fn_with_state(
            gcra_limiter,
            rate_limiter::gcra_rate_limit,
        ))
        .layer(axum::middleware::from_fn(middleware::security_headers))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state.clone());

    (app, state)
}

/// Start the OpenCarrier daemon: boot kernel + HTTP API server.
///
/// This function blocks until Ctrl+C or a shutdown request.
pub async fn run_daemon(
    kernel: OpenCarrierKernel,
    listen_addr: &str,
    daemon_info_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = listen_addr.parse()?;

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    kernel.start_background_agents();

    // ── Plugin loading ──────────────────────────────────────────────
    let plugin_manager = if let Some(ref plugins_dir) = kernel.config.plugins_dir {
        // Resolve: absolute path as-is, relative path joined to opencarrier home_dir
        let resolved = if plugins_dir.is_absolute() {
            plugins_dir.clone()
        } else {
            kernel.config.home_dir.join(plugins_dir)
        };
        if resolved.exists() {
            let kernel_handle: Arc<dyn opencarrier_runtime::kernel_handle::KernelHandle> = kernel.clone();
            let mut pm = PluginManager::new(kernel_handle);
            pm.load_all(&resolved);
            pm.start(&resolved).await;

            // Inject tool dispatcher into kernel
            {
                let dispatcher = pm.tool_dispatcher();
                let mut guard = kernel.plugin_tool_dispatcher.lock().unwrap();
                *guard = Some(dispatcher);
            }

            let tool_count = pm.tool_definitions().len();
            info!("Plugins loaded: {} tools available", tool_count);
            Some(pm)
        } else {
            info!("Plugin directory does not exist, skipping: {}", resolved.display());
            None
        }
    } else {
        None
    };

    // Config file hot-reload watcher (polls every 30 seconds)
    {
        let k = kernel.clone();
        let config_path = kernel.config.home_dir.join("config.toml");
        tokio::spawn(async move {
            let mut last_modified = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let current = std::fs::metadata(&config_path)
                    .and_then(|m| m.modified())
                    .ok();
                if current != last_modified && current.is_some() {
                    last_modified = current;
                    tracing::info!("Config file changed, reloading...");
                    match k.reload_config() {
                        Ok(plan) => {
                            if plan.has_changes() {
                                tracing::info!("Config hot-reload applied: {:?}", plan.hot_actions);
                            } else {
                                tracing::debug!("Config hot-reload: no actionable changes");
                            }
                        }
                        Err(e) => tracing::warn!("Config hot-reload failed: {e}"),
                    }
                }
            }
        });
    }

    let (app, state) = build_router(kernel.clone(), addr, plugin_manager).await;

    // Write daemon info file
    if let Some(info_path) = daemon_info_path {
        // Check if another daemon is already running with this PID file
        if info_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(info_path) {
                if let Ok(info) = serde_json::from_str::<DaemonInfo>(&existing) {
                    // PID alive AND the health endpoint responds → truly running
                    if is_process_alive(info.pid) && is_daemon_responding(&info.listen_addr) {
                        return Err(format!(
                            "Another daemon (PID {}) is already running at {}",
                            info.pid, info.listen_addr
                        )
                        .into());
                    }
                }
            }
            // Stale PID file (process dead or different process reused PID), remove it
            info!("Removing stale daemon info file");
            let _ = std::fs::remove_file(info_path);
        }

        let daemon_info = DaemonInfo {
            pid: std::process::id(),
            listen_addr: addr.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&daemon_info) {
            let _ = std::fs::write(info_path, json);
            // SECURITY: Restrict daemon info file permissions (contains PID and port).
            restrict_permissions(info_path);
        }
    }

    info!("OpenCarrier API server listening on http://{addr}");
    info!("WebChat UI available at http://{addr}/",);
    info!("WebSocket endpoint: ws://{addr}/api/agents/{{id}}/ws",);

    // Use SO_REUSEADDR to allow binding immediately after reboot (avoids TIME_WAIT).
    let socket = socket2::Socket::new(
        if addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        },
        socket2::Type::STREAM,
        None,
    )?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    let listener = tokio::net::TcpListener::from_std(std::net::TcpListener::from(socket))?;

    // Run server with graceful shutdown.
    // SECURITY: `into_make_service_with_connect_info` injects the peer
    // SocketAddr so the auth middleware can check for loopback connections.
    let api_shutdown = state.shutdown_notify.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(api_shutdown))
    .await?;

    // Clean up daemon info file
    if let Some(info_path) = daemon_info_path {
        let _ = std::fs::remove_file(info_path);
    }

    // Shutdown kernel
    kernel.shutdown();

    info!("OpenCarrier daemon stopped");
    Ok(())
}

/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// Read daemon info from the standard location.
pub fn read_daemon_info(home_dir: &Path) -> Option<DaemonInfo> {
    let info_path = home_dir.join("daemon.json");
    let contents = std::fs::read_to_string(info_path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Wait for an OS termination signal OR an API shutdown request.
///
/// On Unix: listens for SIGINT, SIGTERM, and API notify.
/// On Windows: listens for Ctrl+C and API notify.
async fn shutdown_signal(api_shutdown: Arc<tokio::sync::Notify>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), shutting down...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl+C received, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }
}

/// Check if a process with the given PID is still alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Use kill -0 to check if process exists without sending a signal
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        // tasklist /FI "PID eq N" returns "INFO: No tasks..." when no match,
        // or a table row with the PID when found. Check exit code and that
        // "INFO:" is NOT in the output to confirm the process exists.
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| {
                o.status.success() && {
                    let out = String::from_utf8_lossy(&o.stdout);
                    !out.contains("INFO:") && out.contains(&pid.to_string())
                }
            })
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Check if an OpenCarrier daemon is actually responding at the given address.
/// This avoids false positives where a different process reused the same PID
/// after a system reboot.
fn is_daemon_responding(addr: &str) -> bool {
    // Quick TCP connect check — don't make a full HTTP request to avoid delays
    let addr_only = addr
        .strip_prefix("http://")
        .or_else(|| addr.strip_prefix("https://"))
        .unwrap_or(addr);
    if let Ok(sock_addr) = addr_only.parse::<std::net::SocketAddr>() {
        std::net::TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_millis(500))
            .is_ok()
    } else {
        // Fallback: try connecting to hostname
        std::net::TcpStream::connect(addr_only)
            .map(|_| true)
            .unwrap_or(false)
    }
}
