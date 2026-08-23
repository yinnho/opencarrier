//! OpenCarrier daemon server — boots the kernel and serves the HTTP API.

use crate::middleware;
use crate::rate_limiter;
use crate::routes;
use crate::routes::state::AppState;
use crate::webchat;
use crate::ws;
use axum::Router;
use kernel::CarrierKernel;
use runtime::channel_manager::ChannelManager;
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
    kernel: Arc<CarrierKernel>,
    listen_addr: SocketAddr,
    channel_manager: Option<ChannelManager>,
) -> (Router<()>, Arc<AppState>) {
    let state = Arc::new(AppState {
        kernel: kernel.clone(),
        started_at: Instant::now(),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        channel_manager: channel_manager.map(|cm| Arc::new(tokio::sync::Mutex::new(cm))),
    });

    let cors = if state.kernel.config.api_key.trim().is_empty() {
        let port = listen_addr.port();
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            format!("http://localhost:{port}").parse().unwrap(),
            "https://file.yinnho.cn".parse().unwrap(),
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
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::header::HeaderName::from_static("x-api-key"),
            ])
    } else {
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            "http://localhost:4200".parse().unwrap(),
            "http://127.0.0.1:4200".parse().unwrap(),
            "http://localhost:8080".parse().unwrap(),
            "http://127.0.0.1:8080".parse().unwrap(),
            "https://file.yinnho.cn".parse().unwrap(),
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
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::header::HeaderName::from_static("x-api-key"),
            ])
    };

    let api_key = state.kernel.config.api_key.trim().to_string();
    let auth_state = crate::middleware::AuthState {
        api_key: api_key.clone(),
        auth_enabled: state.kernel.config.auth.enabled,
    };
    let gcra_limiter = rate_limiter::create_rate_limiter();

    // Pages router — server-side rendered HTML, handles its own auth checks
    let pages_router = Router::new()
        .route("/", axum::routing::get(crate::pages::overview_page))
        .route("/login", axum::routing::get(crate::pages::login_page))
        .route("/logout", axum::routing::get(crate::pages::logout_page))
        .route("/agents", axum::routing::get(crate::pages::agents_page))
        .route(
            "/agents/{id}",
            axum::routing::get(crate::pages::agent_detail_page),
        )
        .route(
            "/agents/{id}/users/{sender_id}",
            axum::routing::get(crate::pages::user_chat_page),
        )
        .route("/tasks", axum::routing::get(crate::pages::tasks_page))
        .route("/brain", axum::routing::get(crate::pages::brain_page))
        .with_state(state.clone());

    // API router — JSON endpoints protected by auth middleware
    let api_router = Router::new()
        .merge(routes::agents::router())
        .merge(routes::agent_channels::router())
        .merge(routes::auth::router())
        .merge(routes::bots::router())
        .merge(routes::brain::router())
        .merge(routes::clones::router())
        .merge(routes::clone_admins::router())
        .merge(routes::comms::router())
        .merge(routes::config::router())
        .merge(routes::cron::router())
        .merge(routes::dup::router())
        .merge(routes::files::router())
        .merge(routes::hub::router())
        .merge(routes::kv::router())
        .merge(routes::plugins::router())
        .merge(routes::messaging::router())
        .merge(routes::observability::router())
        .merge(routes::sessions::router())
        .merge(routes::tools_skills::router())
        .merge(routes::webhooks::router())
        .merge(routes::weixin::router())
        .merge(routes::weixin_oa::router())
        .merge(routes::wechat_oa::router())
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route("/api/commands", axum::routing::get(routes::list_commands))
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .route("/api/status", axum::routing::get(routes::status))
        .route("/api/version", axum::routing::get(routes::version))
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    // Static assets (public)
    let static_router = Router::new()
        .route("/logo.png", axum::routing::get(webchat::logo_png))
        .route("/favicon.ico", axum::routing::get(webchat::favicon_ico))
        .route("/manifest.json", axum::routing::get(webchat::manifest_json))
        .route("/share", axum::routing::get(webchat::share_page))
        .route("/sw.js", axum::routing::get(webchat::sw_js))
        .route(
            "/bd00e4fe4983179012e6ffdcc66d0c4b.txt",
            axum::routing::get(webchat::verification_txt),
        )
        .route(
            "/katex-fonts/{name}",
            axum::routing::get(webchat::katex_font),
        )
        .route("/v/{phone}", axum::routing::get(webchat::vcard_redirect))
        .with_state(state.clone());

    let app = Router::new()
        .merge(pages_router)
        .merge(api_router)
        .merge(static_router)
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
    kernel: CarrierKernel,
    listen_addr: &str,
    daemon_info_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = listen_addr.parse()?;

    // SECURITY: Refuse to start without API key on non-loopback addresses
    if kernel.config.api_key.trim().is_empty() && !addr.ip().is_loopback() {
        return Err(
            "API key must be set when listening on non-loopback address. \
                     Set api_key in config.toml or bind to 127.0.0.1"
                .into(),
        );
    }

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    kernel.start_background_agents();

    // ── Channel loading ──────────────────────────────────────────────
    let channel_manager = {
        let kernel_handle: Arc<dyn runtime::kernel_handle::KernelHandle> = kernel.clone();
        let mut cm = ChannelManager::new(kernel_handle);

        // Always create sender-based router (auto-assigns first agent for new senders)
        {
            let router = std::sync::Arc::new(runtime::plugin::router::SenderRouter::new(
                &kernel.config.home_dir,
            ));
            // Migrate any routes stored as UUIDs to agent names
            router.migrate_uuid_to_names(|uuid| {
                uuid.parse::<types::agent::AgentId>()
                    .ok()
                    .and_then(|id| kernel.registry.get(id).map(|e| e.manifest.name.clone()))
            });
            cm.set_sender_router(router);
            info!("Sender-based routing enabled");
        }

        // Wire up cron delivery store (last-channel tracking + buffered notifications)
        {
            let store = std::sync::Arc::new(kernel.memory.cron_delivery().clone());
            cm.set_cron_delivery(store);
        }

        // Wire up notify route store (DB-backed notification routing)
        {
            let store = std::sync::Arc::new(kernel.memory.notify_store().clone());
            cm.set_notify_store(store);
        }

        // Migrate old plugins/{platform}/bot/*.toml → {platform}-sessions/*.json
        crate::migration::migrate_bot_toml_to_sessions(&kernel.config.home_dir);
        // Migrate {platform}-sessions/*.json → senders/{sender_id}/session.json
        crate::migration::migrate_sessions_to_senders(&kernel.config.home_dir);
        // Migrate senders/{owner}/{agent}/ → workspaces/{agent}/{owner}/
        crate::migration::migrate_sender_data_to_workspaces(&kernel.config.home_dir);
        // Migrate workspaces/{agent}/{sender}/ → workspaces/{agent}/senders/{sender}/
        crate::migration::migrate_sender_data_to_senders_dir(&kernel.config.home_dir);

        // Register channel adapters from independent crates
        cm.register("feishu", Box::new(channel_feishu::SessionWatcher::new()));
        {
            // wecom: register route_key (session name/bot_id) → bind_agent so
            // inbound wecom messages (App/Kf/SmartBot) reach the bound agent.
            let watcher = channel_wecom::SessionWatcher::new();
            for (bot_id, agent) in watcher.route_mappings() {
                let agent_ref = if let Ok(id) = agent.parse::<types::agent::AgentId>() {
                    kernel
                        .registry
                        .get(id)
                        .map(|e| e.manifest.name.clone())
                        .unwrap_or_else(|| agent.clone())
                } else {
                    agent.clone()
                };
                if cm.get_sender_route(&bot_id).is_none() {
                    cm.set_sender_route(&bot_id, &agent_ref);
                    info!(
                        bot_id = %bot_id,
                        agent = %agent_ref,
                        "wecom: registered route from bind_agent"
                    );
                }
            }
            cm.register("wecom", Box::new(watcher));
        }
        cm.register("weixin", Box::new(channel_weixin::SessionWatcher::new()));
        cm.register(
            "dingtalk",
            Box::new(channel_dingtalk::SessionWatcher::new()),
        );

        // weixin-oa: webhook-based WeChat Official Account channel.
        // Loads senders/{app_id}/session.json files and exposes the OA reply
        // capability (Customer Service Message API).
        {
            let watcher = channel_weixin_oa::SessionWatcher::new();
            watcher.load_from_dir(&kernel.config.home_dir.join("senders"));
            // Register route_key (app_id) → bind_agent so inbound messages
            // reach the bound agent. All followers of one OA share one agent.
            for (app_id, agent) in watcher.route_mappings() {
                // Resolve a UUID bind_agent to an agent name for consistency
                let agent_ref = if let Ok(id) = agent.parse::<types::agent::AgentId>() {
                    kernel
                        .registry
                        .get(id)
                        .map(|e| e.manifest.name.clone())
                        .unwrap_or_else(|| agent.clone())
                } else {
                    agent.clone()
                };
                if cm.get_sender_route(&app_id).is_none() {
                    cm.set_sender_route(&app_id, &agent_ref);
                    info!(
                        app_id = %app_id,
                        agent = %agent_ref,
                        "weixin-oa: registered route from session bind_agent"
                    );
                }
            }
            cm.register("weixin-oa", Box::new(watcher));
        }

        // Register weixin tools with the tool dispatcher
        // (tools will be migrated to MCP servers in Phase 4)
        {
            let dispatcher = cm.tool_dispatcher();
            let mut builtin = runtime::plugin::BuiltinPlugin::new(
                "weixin".to_string(),
                "1.0.0".to_string(),
                std::path::PathBuf::new(),
            );
            builtin.register_tool(Box::new(channel_weixin::WeixinQrLoginTool));
            builtin.register_tool(Box::new(channel_weixin::WeixinSendMessageTool));
            builtin.register_tool(Box::new(channel_weixin::WeixinSendImageTool));
            builtin.register_tool(Box::new(channel_weixin::WeixinSendVideoTool));
            builtin.register_tool(Box::new(channel_weixin::WeixinStatusTool));
            dispatcher.register(std::sync::Arc::new(builtin));
        }

        // Register weixin-oa publish article tool.
        // (charter_create_order is now a config-driven api_tool in the 86助手
        // workspace's api_tools.toml - no longer a hardcoded client-specific
        // ToolProvider. Rich content delivery (mini-program cards, images,
        // files) goes through the unified `Channel::deliver` path and
        // `[DELIVER:key]` markers.)
        {
            let dispatcher = cm.tool_dispatcher();
            let mut builtin = runtime::plugin::BuiltinPlugin::new(
                "weixin-oa".to_string(),
                "1.0.0".to_string(),
                std::path::PathBuf::new(),
            );
            builtin.register_tool(Box::new(channel_weixin_oa::WeixinOaDraftListTool));
            builtin.register_tool(Box::new(channel_weixin_oa::WeixinOaPublishArticleTool));
            dispatcher.register(std::sync::Arc::new(builtin));
        }

        // Inject DB-backed persistence callbacks into WeixinState BEFORE cm.start():
        // weixin channel's start() calls load_from_dir() which needs `sessions_load`
        // already set, otherwise it falls back to scanning JSON files and ignores the
        // weixin_sessions table entirely. This used to run *after* cm.start(), so load
        // always took the JSON path — the DB table existed but was never read on boot.
        {
            let store = kernel.memory.weixin_store().clone();
            let persist_fn: channel_weixin::token::SessionPersistFn =
                std::sync::Arc::new(move |tf| {
                    let row = memory::weixin_store::WeixinSessionRow {
                        channel: tf.channel.clone(),
                        sender_key: tf.sender_key.clone(),
                        bot_id: tf.bot_id.clone(),
                        bot_token: tf.bot_token.clone(),
                        baseurl: tf.baseurl.clone(),
                        ilink_bot_id: tf.ilink_bot_id.clone(),
                        user_id: tf.user_id.clone(),
                        expires_at: tf.expires_at,
                        bind_agent: tf.bind_agent.clone(),
                        context_tokens: serde_json::to_string(&tf.context_tokens)
                            .unwrap_or_default(),
                    };
                    if let Err(e) = store.upsert(&row) {
                        tracing::warn!("Failed to persist weixin session to DB: {e}");
                    }
                });
            let store2 = kernel.memory.weixin_store().clone();
            let load_fn: channel_weixin::token::SessionsLoadFn =
                std::sync::Arc::new(move || match store2.load_all() {
                    Ok(rows) => rows
                        .into_iter()
                        .map(|r| {
                            let ctx: std::collections::HashMap<String, String> =
                                serde_json::from_str(&r.context_tokens).unwrap_or_default();
                            channel_weixin::models::BotTokenFile {
                                channel: r.channel,
                                sender_key: r.sender_key,
                                bot_id: r.bot_id,
                                bot_token: r.bot_token,
                                baseurl: r.baseurl,
                                ilink_bot_id: r.ilink_bot_id,
                                user_id: r.user_id,
                                expires_at: r.expires_at,
                                bind_agent: r.bind_agent,
                                context_tokens: ctx,
                            }
                        })
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        tracing::warn!("Failed to load weixin sessions from DB: {e}");
                        Vec::new()
                    }
                });
            channel_weixin::token::WEIXIN_STATE.set_persist_fns(persist_fn, load_fn);
            info!("WeixinState DB persistence callbacks installed");
        }

        cm.start().await;

        // Start API tool cron scheduler (for tools with [tool.cron] section)
        {
            let home_dir = kernel.config.home_dir.clone();
            let api_tool_configs = runtime::api_tools::loader::load_all_api_tools(&home_dir, None);
            runtime::api_tools::register_cron_tools(api_tool_configs, home_dir).await;
        }

        // Inject channel send / proactive-push probe into kernel for cron delivery
        {
            let send_fn = cm.make_channel_send_fn();
            *kernel.channel_send_fn.write().unwrap() = Some(send_fn);
            let deliver_fn = cm.make_channel_deliver_fn();
            *kernel.channel_deliver_fn.write().unwrap() = Some(deliver_fn);
            let probe = cm.make_supports_proactive_fn();
            *kernel.channel_supports_proactive_fn.write().unwrap() = Some(probe);
        }

        // Inject tool dispatcher into kernel
        {
            let dispatcher = cm.tool_dispatcher();
            let mut guard = kernel
                .plugins
                .plugin_tool_dispatcher
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = Some(dispatcher);
        }

        // Register WeChat bindings from token files
        {
            let token_dir = kernel.config.home_dir.join("weixin-sessions");
            if token_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&token_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("json") {
                            continue;
                        }
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(tf) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let (Some(bot_id), Some(agent)) = (
                                    tf.get("bot_id").and_then(|v| v.as_str()),
                                    tf.get("bind_agent").and_then(|v| v.as_str()),
                                ) {
                                    if !agent.is_empty() {
                                        // Resolve UUID bind_agent to agent name
                                        let agent_ref = if let Ok(id) =
                                            agent.parse::<types::agent::AgentId>()
                                        {
                                            kernel
                                                .registry
                                                .get(id)
                                                .map(|e| e.manifest.name.clone())
                                                .unwrap_or_else(|| agent.to_string())
                                        } else {
                                            agent.to_string()
                                        };
                                        if let Some(uid) =
                                            tf.get("user_id").and_then(|v| v.as_str())
                                        {
                                            if !uid.is_empty() && cm.get_sender_route(uid).is_none()
                                            {
                                                cm.set_sender_route(uid, &agent_ref);
                                                info!(
                                                    bot = %bot_id,
                                                    agent = %agent_ref,
                                                    "Registered WeChat binding from token file (no existing route)"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let tool_count = cm.tool_definitions().len();
        info!("Channels loaded: {} tools available", tool_count);
        Some(cm)
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

    let (app, state) = build_router(kernel.clone(), addr, channel_manager).await;

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

/// Check if an Carrier daemon is actually responding at the given address.
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
