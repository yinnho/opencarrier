//! OpenCarrier daemon server — boots the kernel and serves the HTTP API.

use crate::middleware;
use crate::rate_limiter;
use crate::routes;
use crate::routes::state::AppState;
use crate::webchat;
use crate::ws;
use axum::Router;
use opencarrier_kernel::{KernelHandle, OpenCarrierKernel};
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

    let cors = if state.kernel.config.api_key.trim().is_empty() {
        let port = listen_addr.port();
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            format!("http://localhost:{port}").parse().unwrap(),
        ];
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
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            "http://localhost:4200".parse().unwrap(),
            "http://127.0.0.1:4200".parse().unwrap(),
            "http://localhost:8080".parse().unwrap(),
            "http://127.0.0.1:8080".parse().unwrap(),
        ];
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
        .route("/share", axum::routing::get(webchat::share_page))
        .route("/sw.js", axum::routing::get(webchat::sw_js))
        .route(
            "/katex-fonts/{name}",
            axum::routing::get(webchat::katex_font),
        )
        .merge(routes::agents::router())
        .merge(routes::auth::router())
        .merge(routes::bindings::router())
        .merge(routes::bots::router())
        .merge(routes::brain::router())
        .merge(routes::clones::router())
        .merge(routes::comms::router())
        .merge(routes::config::router())
        .merge(routes::cron::router())
        .merge(routes::files::router())
        .merge(routes::hub::router())
        .merge(routes::kv::router())
        .merge(routes::onboard::router())
        .merge(routes::plugins::router())
        .merge(routes::messaging::router())
        .merge(routes::observability::router())
        .merge(routes::providers::router())
        .merge(routes::sessions::router())
        .merge(routes::tenants::router())
        .merge(routes::tools_skills::router())
        .merge(routes::webhooks::router())
        .merge(routes::weixin::router())
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route("/api/commands", axum::routing::get(routes::list_commands))
        // plugins routes handled by routes::plugins::router() above
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .route("/api/status", axum::routing::get(routes::status))
        .route("/api/version", axum::routing::get(routes::version))
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024))
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
            let kernel_handle: Arc<dyn opencarrier_runtime::kernel_handle::KernelHandle> =
                kernel.clone();
            let mut pm = PluginManager::new(kernel_handle);
            let mut registry = opencarrier_runtime::plugin::BuiltinPluginRegistry::new();
            // Register built-in WeChat channel adapters and tools
            registry.register_channel("weixin", || {
                Box::new(opencarrier_runtime::plugin::channels::weixin::TenantWatcher::new())
            });
            registry.register_tool("weixin", || Box::new(opencarrier_runtime::plugin::channels::weixin::WeixinQrLoginTool));
            registry.register_tool("weixin", || Box::new(opencarrier_runtime::plugin::channels::weixin::WeixinSendMessageTool));
            registry.register_tool("weixin", || Box::new(opencarrier_runtime::plugin::channels::weixin::WeixinStatusTool));

            // Register built-in WeCom channel adapters and tools
            registry.register_channel("wecom", || {
                Box::new(opencarrier_runtime::plugin::channels::wecom::WeComAppKfWatcher::new())
            });
            registry.register_channel("wecom", || {
                Box::new(opencarrier_runtime::plugin::channels::wecom::WeComSmartBotWatcher::new())
            });
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::SendMessageTool));
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::BotGenerateTool));
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::BotPollTool));
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::QrCodeTool));
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::BotRegisterTool));
            registry.register_tool("wecom", || Box::new(opencarrier_runtime::plugin::channels::wecom::BotBindTool));
            opencarrier_runtime::plugin::channels::wecom::mcp::register_mcp_tools(&mut registry);

            // Register built-in Feishu channel adapter and tools
            registry.register_channel("feishu", || {
                Box::new(opencarrier_runtime::plugin::channels::feishu::FeishuWatcher::new())
            });
            opencarrier_runtime::plugin::channels::feishu::tools::register_feishu_tools(&mut registry);

            pm.load_all(&resolved, &registry);
            pm.start(&resolved).await;

            // Inject tool dispatcher into kernel
            {
                let dispatcher = pm.tool_dispatcher();
                let mut guard = kernel.plugins.plugin_tool_dispatcher.lock().unwrap();
                *guard = Some(dispatcher);
            }

            // Register WeChat bindings from token files (weixin uses token files,
            // not bot.toml, so the plugin manager never discovers them at startup)
            {
                let token_dir = kernel.config.home_dir.join("weixin-tokens");
                if token_dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(&token_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                                continue;
                            }
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if let Ok(tf) =
                                    serde_json::from_str::<serde_json::Value>(&content)
                                {
                                    if let (Some(name), Some(agent)) = (
                                        tf.get("name").and_then(|v| v.as_str()),
                                        tf.get("bind_agent").and_then(|v| v.as_str()),
                                    ) {
                                        if uuid::Uuid::parse_str(agent).is_ok() {
                                            pm.add_channel_binding("weixin", name, agent);
                                            kernel.set_default_plugin_tenant(agent, name);
                                            info!(
                                                tenant = %name,
                                                agent = %agent,
                                                "Registered WeChat binding from token file"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let tool_count = pm.tool_definitions().len();
            info!("Plugins loaded: {} tools available", tool_count);
            Some(pm)
        } else {
            info!(
                "Plugin directory does not exist, skipping: {}",
                resolved.display()
            );
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
