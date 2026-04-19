//! OpenCarrier CLI — command-line interface for the OpenCarrier Agent OS.
//!
//! When a daemon is running (`opencarrier start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

mod dotenv;
mod launcher;
mod mcp;
pub mod progress;
pub mod serve;
mod acp;
pub mod table;
mod templates;
mod tui;
mod ui;

use clap::{Parser, Subcommand};
use colored::Colorize;
use opencarrier_api::server::read_daemon_info;
use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_types::agent::{AgentId, AgentManifest};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
#[cfg(windows)]
use std::sync::atomic::Ordering;

/// Global flag set by the Ctrl+C handler.
static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);

/// Install a Ctrl+C handler that force-exits the process.
/// On Windows/MINGW, the default handler doesn't reliably interrupt blocking
/// `read_line` calls, so we explicitly call `process::exit`.
fn install_ctrlc_handler() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
            if CTRLC_PRESSED.swap(true, Ordering::SeqCst) {
                // Second press: hard exit
                std::process::exit(130);
            }
            // First press: print message and exit cleanly
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\nInterrupted.\n");
            std::process::exit(0);
        }
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    #[cfg(not(windows))]
    {
        // On Unix, the default SIGINT handler already interrupts read_line
        // and terminates the process.
        let _ = &CTRLC_PRESSED;
    }
}

const AFTER_HELP: &str = "\
\x1b[1mQuick Start:\x1b[0m
  yinghe                    直接启动

\x1b[1;36mExamples:\x1b[0m
  yinghe agent new coder      创建新的 coder agent
  yinghe models list          查看可用模型
  yinghe doctor               运行诊断检查

\x1b[1;36mMore:\x1b[0m
  Dashboard:  http://127.0.0.1:4200/ (when running)";

/// OpenCarrier — the open-source Agent Operating System.
#[derive(Parser)]
#[command(
    name = "opencarrier",
    version,
    about = "\u{1F40D} OpenCarrier \u{2014} Open-source Agent Operating System",
    long_about = "\u{1F40D} OpenCarrier \u{2014} Open-source Agent Operating System\n\n\
                  Deploy, manage, and orchestrate AI agents from your terminal.\n\
                  50+ models \u{00b7} infinite possibilities.",
    after_help = AFTER_HELP,
)]
struct Cli {
    /// Path to config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            config: None,
            command: Some(Commands::Start),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Start the carrier (default command).
    #[command(name = "start", alias = "s")]
    Start,
    /// Initialize OpenCarrier (create ~/.opencarrier/ and default config).
    Init {
        /// Quick mode: no prompts, just write config + .env (for CI/scripts).
        #[arg(long)]
        quick: bool,
    },
    /// Stop the running daemon.
    Stop,
    /// Manage agents (new, list, chat, kill, spawn) [*].
    #[command(subcommand)]
    Agent(AgentCommands),
    /// Show or edit configuration (show, edit, get, set, keys) [*].
    #[command(subcommand)]
    Config(ConfigCommands),
    /// Chat with a specific agent (分身).
    Chat {
        /// Agent name or ID to chat with.
        agent: String,
    },
    /// Show kernel status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Run diagnostic health checks.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Attempt to auto-fix issues (create missing dirs/config).
        #[arg(long)]
        repair: bool,
    },
    /// Open the web dashboard in the default browser.
    Dashboard,
    /// Generate shell completion scripts.
    Completion {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Start MCP (Model Context Protocol) server over stdio.
    Mcp,
    /// ACP mode (auto-detected: used when stdin is a pipe, not a terminal).
    #[command(hide = true)]
    Acp,
    /// Launch the interactive terminal dashboard.
    Tui,
    /// Browse models, aliases, and providers [*].
    #[command(subcommand)]
    Models(ModelsCommands),
        /// Manage scheduled jobs (list, create, delete, enable, disable) [*].
    #[command(subcommand)]
    Cron(CronCommands),
    /// List conversation sessions.
    Sessions {
        /// Optional agent name or ID to filter by.
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Tail the OpenCarrier log file.
    Logs {
        /// Number of lines to show.
        #[arg(long, default_value = "50")]
        lines: usize,
        /// Follow log output in real time.
        #[arg(long, short)]
        follow: bool,
    },
    /// Security tools and audit trail [*].
    #[command(subcommand)]
    Security(SecurityCommands),
    /// Search and manage agent memory (KV store) [*].
    #[command(subcommand)]
    Memory(MemoryCommands),
    /// Send a one-shot message to an agent.
    Message {
        /// Agent name or ID.
        agent: String,
        /// Message text. Reads from stdin if omitted or "-".
        text: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Reset local config and state.
    Reset {
        /// Skip confirmation prompt.
        #[arg(long)]
        confirm: bool,
    },
    /// Completely uninstall OpenCarrier from your system.
    Uninstall {
        /// Skip confirmation prompt (also --yes).
        #[arg(long, alias = "yes")]
        confirm: bool,
        /// Keep config files (config.toml, .env, secrets.env).
        #[arg(long)]
        keep_config: bool,
    },
    /// Configure LLM providers (interactive API key setup).
    Providers,
    /// Hub operations — search and install clones from openclone-hub.
    Hub {
        #[command(subcommand)]
        sub: HubCommands,
    },
}

#[derive(Subcommand)]
enum HubCommands {
    /// Search templates on Hub.
    Search {
        /// Search query (name, description, tags).
        query: Option<String>,
    },
    /// Download and install a clone from Hub.
    Install {
        /// Template name to install.
        name: String,
        /// Specific version (default: latest).
        #[arg(short, long)]
        version: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show the current configuration.
    Show,
    /// Open the configuration file in your editor.
    Edit,
    /// Get a config value by dotted key path (e.g. "default_model.provider").
    Get {
        /// Dotted key path (e.g. "default_model.provider", "api_listen").
        key: String,
    },
    /// Set a config value (warning: strips TOML comments).
    Set {
        /// Dotted key path.
        key: String,
        /// New value.
        value: String,
    },
    /// Remove a config key (warning: strips TOML comments).
    Unset {
        /// Dotted key path to remove (e.g. "api.cors_origin").
        key: String,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Spawn a new agent from a template (interactive or by name).
    New {
        /// Template name (e.g., "coder", "assistant"). Interactive picker if omitted.
        template: Option<String>,
    },
    /// Spawn a new agent from a manifest file.
    Spawn {
        /// Path to the agent manifest TOML file.
        manifest: PathBuf,
    },
    /// List all running agents.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Interactive chat with an agent.
    Chat {
        /// Agent ID (UUID) or name.
        agent_id: String,
        /// Send a single message non-interactively, print the response, and exit.
        #[arg(short, long)]
        message: Option<String>,
        /// Read file content and prepend to message. Use "-" for stdin.
        /// Can be specified multiple times.
        #[arg(short, long)]
        file: Vec<String>,
    },
    /// Kill an agent.
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
}

#[derive(Subcommand)]
enum ModelsCommands {
    /// List available models (optionally filter by provider).
    List {
        /// Filter by provider name.
        #[arg(long)]
        provider: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show model aliases (shorthand names).
    Aliases {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set the default model for the daemon.
    Set {
        /// Model ID or alias (e.g. "gpt-4o", "claude-sonnet"). Interactive picker if omitted.
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    /// List scheduled jobs.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new scheduled job.
    Create {
        /// Agent name or ID to run.
        agent: String,
        /// Cron expression (e.g. "0 */6 * * *").
        spec: String,
        /// Prompt to send when the job fires.
        prompt: String,
        /// Optional job name (auto-generated if omitted).
        #[arg(long)]
        name: Option<String>,
    },
    /// Delete a scheduled job.
    Delete {
        /// Job ID.
        id: String,
    },
    /// Enable a disabled job.
    Enable {
        /// Job ID.
        id: String,
    },
    /// Disable a job without deleting it.
    Disable {
        /// Job ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum SecurityCommands {
    /// Show security status summary.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show recent audit trail entries.
    Audit {
        /// Maximum number of entries to show.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Verify audit trail integrity (Merkle chain).
    Verify,
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// List KV pairs for an agent.
    List {
        /// Agent name or ID.
        agent: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Get a specific KV value.
    Get {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set a KV value.
    Set {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete a KV pair.
    Delete {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
    },
}


fn config_log_level() -> String {
    let config_path = if let Ok(home) = std::env::var("OPENCARRIER_HOME") {
        std::path::PathBuf::from(home).join("config.toml")
    } else {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".opencarrier")
            .join("config.toml")
    };
    if let Ok(content) = std::fs::read_to_string(config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("log_level") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    let level = val.trim().trim_matches('"').trim_matches('\'');
                    if !level.is_empty() {
                        return level.to_string();
                    }
                }
            }
        }
    }
    "info".to_string()
}

fn init_tracing_stderr() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config_log_level())),
        )
        .with_writer(std::io::stderr)
        .init();
}

/// Get the OpenCarrier home directory, respecting OPENCARRIER_HOME env var.
fn cli_opencarrier_home() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("OPENCARRIER_HOME") {
        return std::path::PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".opencarrier")
}

/// Redirect tracing to a log file so it doesn't corrupt the ratatui TUI.
fn init_tracing_file() {
    let log_dir = cli_opencarrier_home();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tui.log");

    match std::fs::File::create(&log_path) {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config_log_level())),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        Err(_) => {
            // Fallback: suppress all output rather than corrupt the TUI
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::ERROR)
                .with_writer(std::io::sink)
                .init();
        }
    }
}

fn main() {
    // Load ~/.opencarrier/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    let cli = Cli::parse();

    // Determine if this invocation launches a ratatui TUI.
    // TUI modes must NOT install the Ctrl+C handler (it calls process::exit
    // which bypasses ratatui::restore and leaves the terminal in raw mode).
    // TUI modes also need file-based tracing (stderr output corrupts the TUI).
    let is_tui_mode = matches!(cli.command.as_ref(), Some(Commands::Tui))
        || matches!(cli.command.as_ref(), Some(Commands::Chat { .. }))
        || matches!(
            cli.command.as_ref(),
            Some(Commands::Agent(AgentCommands::Chat { .. }))
        );

    if is_tui_mode {
        init_tracing_file();
    } else {
        // CLI subcommands: install Ctrl+C handler for clean interrupt of
        // blocking read_line calls, and trace to stderr.
        install_ctrlc_handler();
        init_tracing_stderr();
    }

    // 默认命令：根据 stdin 类型自动选择
    // - stdin 是 pipe（被外部 spawn）→ ACP 模式
    // - stdin 是 TTY（用户在终端）→ 启动 daemon
    let command = cli.command.unwrap_or_else(|| {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            Commands::Acp
        } else {
            Commands::Start
        }
    });

    match command {
        Commands::Tui => tui::run(cli.config),
        Commands::Init { quick } => cmd_init(quick),
        Commands::Start => cmd_start(cli.config),
        Commands::Stop => cmd_stop(),
        Commands::Agent(sub) => match sub {
            AgentCommands::New { template } => cmd_agent_new(cli.config, template),
            AgentCommands::Spawn { manifest } => cmd_agent_spawn(cli.config, manifest),
            AgentCommands::List { json } => cmd_agent_list(cli.config, json),
            AgentCommands::Chat { agent_id, message, file } => {
                if message.is_some() || !file.is_empty() {
                    let msg = message.unwrap_or_default();
                    cmd_agent_chat_once(cli.config, &agent_id, &msg, &file)
                } else {
                    cmd_agent_chat(cli.config, &agent_id)
                }
            }
            AgentCommands::Kill { agent_id } => cmd_agent_kill(cli.config, &agent_id),
        },
        Commands::Config(sub) => match sub {
            ConfigCommands::Show => cmd_config_show(),
            ConfigCommands::Edit => cmd_config_edit(),
            ConfigCommands::Get { key } => cmd_config_get(&key),
            ConfigCommands::Set { key, value } => cmd_config_set(&key, &value),
            ConfigCommands::Unset { key } => cmd_config_unset(&key),
        },
        Commands::Chat { agent } => cmd_quick_chat(cli.config, agent),
        Commands::Status { json } => cmd_status(cli.config, json),
        Commands::Doctor { json, repair } => cmd_doctor(json, repair),
        Commands::Dashboard => cmd_dashboard(),
        Commands::Completion { shell } => cmd_completion(shell),
        Commands::Mcp => mcp::run_mcp_server(cli.config),
        Commands::Acp => serve::run_acp_mode(cli.config),
        // ── New commands ────────────────────────────────────────────────
        Commands::Models(sub) => match sub {
            ModelsCommands::List { provider, json } => cmd_models_list(provider.as_deref(), json),
            ModelsCommands::Aliases { json } => cmd_models_aliases(json),
            ModelsCommands::Set { model } => cmd_models_set(model),
        },
        Commands::Cron(sub) => match sub {
            CronCommands::List { json } => cmd_cron_list(json),
            CronCommands::Create {
                agent,
                spec,
                prompt,
                name,
            } => cmd_cron_create(&agent, &spec, &prompt, name.as_deref()),
            CronCommands::Delete { id } => cmd_cron_delete(&id),
            CronCommands::Enable { id } => cmd_cron_toggle(&id, true),
            CronCommands::Disable { id } => cmd_cron_toggle(&id, false),
        },
        Commands::Sessions { agent, json } => cmd_sessions(agent.as_deref(), json),
        Commands::Logs { lines, follow } => cmd_logs(lines, follow),
        Commands::Security(sub) => match sub {
            SecurityCommands::Status { json } => cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => cmd_security_audit(limit, json),
            SecurityCommands::Verify => cmd_security_verify(),
        },
        Commands::Memory(sub) => match sub {
            MemoryCommands::List { agent, json } => cmd_memory_list(&agent, json),
            MemoryCommands::Get { agent, key, json } => cmd_memory_get(&agent, &key, json),
            MemoryCommands::Set { agent, key, value } => cmd_memory_set(&agent, &key, &value),
            MemoryCommands::Delete { agent, key } => cmd_memory_delete(&agent, &key),
        },
        Commands::Message { agent, text, json } => {
            let msg = match text {
                Some(t) if t != "-" => t.clone(),
                _ => {
                    let mut buf = String::new();
                    use std::io::Read;
                    std::io::stdin().read_to_string(&mut buf).unwrap_or(0);
                    buf.trim().to_string()
                }
            };
            cmd_message(&agent, &msg, json)
        }
        Commands::Reset { confirm } => cmd_reset(confirm),
        Commands::Uninstall {
            confirm,
            keep_config,
        } => cmd_uninstall(confirm, keep_config),
        Commands::Providers => cmd_providers(),
        Commands::Hub { sub } => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(cmd_hub(sub));
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon detection helpers
// ---------------------------------------------------------------------------

/// Try to find a running daemon. Returns its base URL if found.
/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file_permissions(_path: &std::path::Path) {}

/// SECURITY: Restrict directory permissions to owner-only (0700) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir_permissions(_path: &std::path::Path) {}

pub(crate) fn find_daemon() -> Option<String> {
    let home_dir = cli_opencarrier_home();
    let info = read_daemon_info(&home_dir)?;

    // Normalize listen address: replace 0.0.0.0 with 127.0.0.1 to avoid
    // DNS/connectivity issues on macOS where 0.0.0.0 can hang.
    let addr = info.listen_addr.replace("0.0.0.0", "127.0.0.1");
    let url = format!("http://{addr}/api/health");

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if resp.status().is_success() {
        Some(format!("http://{addr}"))
    } else {
        None
    }
}

/// Build an HTTP client for daemon calls.
///
/// When api_key is configured in config.toml, the client automatically
/// includes a `Authorization: Bearer <key>` header on every request.
/// When api_key is empty or missing, no auth header is sent.
pub(crate) fn daemon_client() -> reqwest::blocking::Client {
    let mut builder =
        reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(120));

    if let Some(key) = read_api_key() {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
        builder = builder.default_headers(headers);
    }

    builder.build().expect("Failed to build HTTP client")
}

/// Helper: send a request to the daemon and parse the JSON body.
/// Exits with error on connection failure.
pub(crate) fn daemon_json(
    resp: Result<reqwest::blocking::Response, reqwest::Error>,
) -> serde_json::Value {
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.json::<serde_json::Value>().unwrap_or_default();
            if status.is_server_error() {
                ui::error_with_fix(
                    &format!("Daemon returned error ({})", status),
                    "Check daemon logs: ~/.opencarrier/tui.log",
                );
            }
            body
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                ui::error_with_fix(
                    "Request timed out",
                    "The agent may be processing a complex request. Try again, or check `opencarrier status`",
                );
            } else if msg.contains("Connection refused") || msg.contains("connect") {
                ui::error_with_fix(
                    "Cannot connect to daemon",
                    "Is the daemon running? Start it with: opencarrier start",
                );
            } else {
                ui::error_with_fix(
                    &format!("Daemon communication error: {msg}"),
                    "Check `opencarrier status` or restart: opencarrier start",
                );
            }
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_init(quick: bool) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Could not determine home directory");
            std::process::exit(1);
        }
    };

    let opencarrier_dir = cli_opencarrier_home();

    // --- Ensure directories exist ---
    if !opencarrier_dir.exists() {
        std::fs::create_dir_all(&opencarrier_dir).unwrap_or_else(|e| {
            ui::error_with_fix(
                &format!("Failed to create {}", opencarrier_dir.display()),
                &format!("Check permissions on {}", home.display()),
            );
            eprintln!("  {e}");
            std::process::exit(1);
        });
        restrict_dir_permissions(&opencarrier_dir);
    }

    for sub in ["data", "agents"] {
        let dir = opencarrier_dir.join(sub);
        if !dir.exists() {
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
                eprintln!("Error creating {sub} dir: {e}");
                std::process::exit(1);
            });
        }
    }

    if quick {
        cmd_init_quick(&opencarrier_dir);
    } else if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        ui::hint("Non-interactive terminal detected — running in quick mode");
        ui::hint("For the interactive wizard, run: opencarrier init (in a terminal)");
        cmd_init_quick(&opencarrier_dir);
    } else {
        cmd_init_interactive(&opencarrier_dir);
    }
}

/// Quick init: no prompts, auto-detect, write config + .env, print next steps.
fn cmd_init_quick(opencarrier_dir: &std::path::Path) {
    ui::banner();
    ui::blank();

    let (provider, api_key_env, model) = detect_best_provider();

    write_config_if_missing(opencarrier_dir, provider, model, api_key_env);

    ui::blank();
    ui::success("OpenCarrier initialized (quick mode)");
    ui::kv("Provider", provider);
    ui::kv("Model", model);
    ui::blank();
    ui::next_steps(&[
        "Start the daemon:  opencarrier start",
        "Chat:              opencarrier chat",
    ]);
}

/// Interactive 5-step onboarding wizard (ratatui TUI).
fn cmd_init_interactive(opencarrier_dir: &std::path::Path) {
    use tui::screens::init_wizard::{self, InitResult, LaunchChoice};

    match init_wizard::run() {
        InitResult::Completed {
            provider,
            model,
            daemon_started,
            launch,
        } => {
            // Print summary after TUI restores terminal
            ui::blank();
            ui::success("OpenCarrier initialized!");
            ui::kv("Provider", &provider);
            ui::kv("Model", &model);

            if daemon_started {
                ui::kv_ok("Daemon", "running");
            }
            ui::blank();

            // Execute the user's chosen launch action.
            match launch {
                LaunchChoice::Desktop => {
                    launch_desktop_app(opencarrier_dir);
                }
                LaunchChoice::Dashboard => {
                    if let Some(base) = find_daemon() {
                        let url = format!("{base}/");
                        ui::success(&format!("Opening dashboard at {url}"));
                        if !open_in_browser(&url) {
                            ui::hint(&format!("Could not open browser. Visit: {url}"));
                        }
                    } else {
                        ui::error("Daemon is not running. Start it with: opencarrier start");
                    }
                }
                LaunchChoice::Chat => {
                    ui::hint("Starting chat session...");
                    ui::blank();
                    // Note: tracing was initialized for stderr (init is a CLI
                    // subcommand).  The chat TUI takes over the terminal with
                    // raw mode so stderr output is suppressed.  We can't
                    // reinitialize tracing (global subscriber is set once).
                    cmd_quick_chat(None, "clone-creator".to_string());
                }
            }
        }
        InitResult::Cancelled => {
            println!("  Setup cancelled.");
        }
    }
}

/// Launch the opencarrier-desktop Tauri app, connecting to the running daemon.
fn launch_desktop_app(_opencarrier_dir: &std::path::Path) {
    // Look for the desktop binary next to our own executable.
    let desktop_bin = {
        let exe = std::env::current_exe().ok();
        let dir = exe.as_ref().and_then(|e| e.parent());

        #[cfg(windows)]
        let name = "opencarrier-desktop.exe";
        #[cfg(not(windows))]
        let name = "opencarrier-desktop";

        dir.map(|d| d.join(name))
    };

    match desktop_bin {
        Some(ref path) if path.exists() => {
            ui::success("Launching OpenCarrier Desktop...");
            match std::process::Command::new(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    ui::success("Desktop app started.");
                }
                Err(e) => {
                    ui::error(&format!("Failed to launch desktop app: {e}"));
                    ui::hint("Try: opencarrier dashboard");
                }
            }
        }
        _ => {
            ui::error("Desktop app not found.");
            ui::hint("Install it with: cargo install opencarrier-desktop");
            ui::hint("Falling back to web dashboard...");
            ui::blank();
            if let Some(base) = find_daemon() {
                let url = format!("{base}/");
                if !open_in_browser(&url) {
                    // Browser launch failed entirely (e.g., sandbox EPERM,
                    // no display server, container environment).
                    ui::hint("Could not open a browser automatically.");
                }
                // Always print the URL so the user can open it manually,
                // even when open_in_browser reported success — the spawned
                // opener may still fail asynchronously.
                ui::hint(&format!("Dashboard: {url}"));
            } else {
                ui::hint("Daemon is not running. Start it with: opencarrier start");
                ui::hint("Then open: http://127.0.0.1:4200");
            }
        }
    }
}

/// Auto-detect the best available provider.
fn detect_best_provider() -> (&'static str, &'static str, &'static str) {
    let providers = provider_list();

    for (p, env_var, m, display) in &providers {
        if std::env::var(env_var).is_ok() {
            ui::success(&format!("Detected {display} ({env_var})"));
            return (p, env_var, m);
        }
    }
    // Also check GOOGLE_API_KEY
    if std::env::var("GOOGLE_API_KEY").is_ok() {
        ui::success("Detected Gemini (GOOGLE_API_KEY)");
        return ("gemini", "GOOGLE_API_KEY", "gemini-2.5-flash");
    }
    // Check if Ollama is running locally (no API key needed)
    if check_ollama_available() {
        ui::success("Detected Ollama running locally (no API key needed)");
        return ("ollama", "OLLAMA_API_KEY", "llama3.2");
    }
    ui::hint("No LLM provider API keys found");
    ui::hint("Groq offers a free tier: https://console.groq.com");
    ui::hint("Or install Ollama for local models: https://ollama.com");
    ("groq", "GROQ_API_KEY", "llama-3.3-70b-versatile")
}

/// Static list of supported providers: (id, env_var, default_model, display_name).
fn provider_list() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("groq", "GROQ_API_KEY", "llama-3.3-70b-versatile", "Groq"),
        ("gemini", "GEMINI_API_KEY", "gemini-2.5-flash", "Gemini"),
        ("deepseek", "DEEPSEEK_API_KEY", "deepseek-chat", "DeepSeek"),
        (
            "anthropic",
            "ANTHROPIC_API_KEY",
            "claude-sonnet-4-20250514",
            "Anthropic",
        ),
        ("openai", "OPENAI_API_KEY", "gpt-4o", "OpenAI"),
        (
            "openrouter",
            "OPENROUTER_API_KEY",
            "openrouter/google/gemini-2.5-flash",
            "OpenRouter",
        ),
    ]
}

/// Quick probe to check if Ollama is running on localhost.
fn check_ollama_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 11434)),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

/// Write config.toml if it doesn't already exist.
fn write_config_if_missing(
    opencarrier_dir: &std::path::Path,
    _provider: &str,
    _model: &str,
    _api_key_env: &str,
) {
    let config_path = opencarrier_dir.join("config.toml");
    if config_path.exists() {
        ui::check_ok(&format!("Config already exists: {}", config_path.display()));
    } else {
        let default_config = r#"# OpenCarrier Agent OS configuration
# See https://github.com/RightNow-AI/opencarrier for documentation

# For Docker, change to "0.0.0.0:4200" or set OPENCARRIER_LISTEN env var.
api_listen = "127.0.0.1:4200"

[brain]
config = "brain.json"

[memory]
decay_rate = 0.05
"#
        .to_string();
        std::fs::write(&config_path, &default_config).unwrap_or_else(|e| {
            ui::error_with_fix("Failed to write config", &e.to_string());
            std::process::exit(1);
        });
        restrict_file_permissions(&config_path);
        ui::success(&format!("Created: {}", config_path.display()));
    }
}

fn cmd_start(config: Option<PathBuf>) {
    if let Some(base) = find_daemon() {
        ui::error_with_fix(
            &format!("Daemon already running at {base}"),
            "Use `opencarrier status` to check it, or stop it first",
        );
        std::process::exit(1);
    }

    ui::banner();
    ui::blank();

    println!("  启动服务中...");
    ui::blank();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let kernel_config = opencarrier_kernel::config::load_config(config.as_deref());

        let kernel = match OpenCarrierKernel::boot_with_config(kernel_config) {
            Ok(k) => k,
            Err(e) => {
                boot_kernel_error(&e);
                std::process::exit(1);
            }
        };

        let listen_addr = kernel.config.api_listen.clone();
        let daemon_info_path = kernel.config.home_dir.join("daemon.json");
        let provider = kernel.config.default_model.provider.clone();
        let model = kernel.config.default_model.model.clone();
        let agent_count = kernel.registry.count();
        let model_count = kernel
            .model_catalog
            .read()
            .map(|c| c.list_models().len())
            .unwrap_or(0);

        ui::success(&format!("Kernel booted ({provider}/{model})"));
        if model_count > 0 {
            ui::success(&format!("{model_count} models available"));
        }
        if agent_count > 0 {
            ui::success(&format!("{agent_count} agent(s) loaded"));
        }
        ui::blank();
        ui::kv("API", &format!("http://{listen_addr}"));
        ui::kv("Dashboard", &format!("http://{listen_addr}/"));
        ui::kv("Modality", &provider);
        ui::kv("Model", &model);
        ui::blank();
        ui::hint("Open the dashboard in your browser, or run `opencarrier chat`");
        ui::hint("Press Ctrl+C to stop the daemon");
        ui::blank();

        if let Err(e) =
            opencarrier_api::server::run_daemon(kernel, &listen_addr, Some(&daemon_info_path)).await
        {
            ui::error(&format!("Daemon error: {e}"));
            std::process::exit(1);
        }

        ui::blank();
        println!("  OpenCarrier daemon stopped.");
    });
}

/// Read the api_key from ~/.opencarrier/config.toml (if any).
///
/// Returns `None` when the key is missing, empty, or whitespace-only —
/// meaning the daemon is running in public (unauthenticated) mode.
fn read_api_key() -> Option<String> {
    // 1. Config file takes precedence
    let config_path = cli_opencarrier_home().join("config.toml");
    if let Ok(text) = std::fs::read_to_string(config_path) {
        if let Ok(table) = text.parse::<toml::Value>() {
            if let Some(key) = table.get("api_key").and_then(|v| v.as_str()) {
                let key = key.trim();
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            }
        }
    }
    // 2. Fall back to OPENCARRIER_API_KEY env var
    if let Ok(key) = std::env::var("OPENCARRIER_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    None
}

fn cmd_stop() {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client.post(format!("{base}/api/shutdown")).send() {
                Ok(r) if r.status().is_success() => {
                    // Wait for daemon to actually stop (up to 5 seconds)
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if find_daemon().is_none() {
                            ui::success("Daemon stopped");
                            return;
                        }
                    }
                    // Still alive — force kill via PID
                    {
                        let of_dir = cli_opencarrier_home();
                        if let Some(info) = read_daemon_info(&of_dir) {
                            force_kill_pid(info.pid);
                            let _ = std::fs::remove_file(of_dir.join("daemon.json"));
                        }
                    }
                    ui::success("Daemon stopped (forced)");
                }
                Ok(r) => {
                    ui::error(&format!("Shutdown request failed ({})", r.status()));
                }
                Err(e) => {
                    ui::error(&format!("Could not reach daemon: {e}"));
                }
            }
        }
        None => {
            ui::warn_with_fix(
                "No running daemon found",
                "Is it running? Check with: opencarrier status",
            );
        }
    }
}

fn force_kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

/// Show context-aware error for kernel boot failures.
fn boot_kernel_error(e: &opencarrier_kernel::error::KernelError) {
    let msg = e.to_string();
    if msg.contains("parse") || msg.contains("toml") || msg.contains("config") {
        ui::error_with_fix(
            "Failed to parse configuration",
            "Check your config.toml syntax: opencarrier config show",
        );
    } else if msg.contains("database") || msg.contains("locked") || msg.contains("sqlite") {
        ui::error_with_fix(
            "Database error (file may be locked)",
            "Check if another OpenCarrier process is running: opencarrier status",
        );
    } else if msg.contains("key") || msg.contains("API") || msg.contains("auth") {
        ui::error_with_fix(
            "LLM provider authentication failed",
            "Run `opencarrier doctor` to check your API key configuration",
        );
    } else {
        ui::error_with_fix(
            &format!("Failed to boot kernel: {msg}"),
            "Run `opencarrier doctor` to diagnose the issue",
        );
    }
}

fn cmd_agent_spawn(config: Option<PathBuf>, manifest_path: PathBuf) {
    if !manifest_path.exists() {
        ui::error_with_fix(
            &format!("Manifest file not found: {}", manifest_path.display()),
            "Use `opencarrier agent new` to spawn from a template instead",
        );
        std::process::exit(1);
    }

    let contents = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        eprintln!("Error reading manifest: {e}");
        std::process::exit(1);
    });

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": contents}))
                .send(),
        );
        if body.get("agent_id").is_some() {
            println!("Agent spawned successfully!");
            println!("  ID:   {}", body["agent_id"].as_str().unwrap_or("?"));
            println!("  Name: {}", body["name"].as_str().unwrap_or("?"));
        } else {
            eprintln!(
                "Failed to spawn agent: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let manifest: AgentManifest = toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("Error parsing manifest: {e}");
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(manifest) {
            Ok(id) => {
                println!("Agent spawned (in-process mode).");
                println!("  ID: {id}");
                println!("\n  Note: Agent will be lost when this process exits.");
                println!("  For persistent agents, use `opencarrier start` first.");
            }
            Err(e) => {
                eprintln!("Failed to spawn agent: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn cmd_agent_list(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/agents")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        let agents = body.as_array();

        match agents {
            Some(agents) if agents.is_empty() => println!("No agents running."),
            Some(agents) => {
                println!(
                    "{:<38} {:<16} {:<10} {:<12} MODEL",
                    "ID", "NAME", "STATE", "MODALITY"
                );
                println!("{}", "-".repeat(95));
                for a in agents {
                    println!(
                        "{:<38} {:<16} {:<10} {:<12} {}",
                        a["id"].as_str().unwrap_or("?"),
                        a["name"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["modality"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    );
                }
            }
            None => println!("No agents running."),
        }
    } else {
        let kernel = boot_kernel(config);
        let agents = kernel.registry.list();

        if json {
            let list: Vec<serde_json::Value> = agents
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&list).unwrap_or_default()
            );
            return;
        }

        if agents.is_empty() {
            println!("No agents running.");
            return;
        }

        println!("{:<38} {:<20} {:<12} CREATED", "ID", "NAME", "STATE");
        println!("{}", "-".repeat(85));
        for entry in agents {
            println!(
                "{:<38} {:<20} {:<12} {}",
                entry.id,
                entry.name,
                format!("{:?}", entry.state),
                entry.created_at.format("%Y-%m-%d %H:%M")
            );
        }
    }
}

fn cmd_agent_chat(config: Option<PathBuf>, agent_id_str: &str) {
    tui::chat_runner::run_chat_tui(config, Some(agent_id_str.to_string()));
}

/// Non-interactive chat: send one message (with optional file attachments), print response, exit.
fn cmd_agent_chat_once(
    config: Option<PathBuf>,
    agent_id_str: &str,
    message: &str,
    files: &[String],
) {
    use opencarrier_kernel::OpenCarrierKernel;
    use opencarrier_types::agent::AgentId;
    use std::io::Read;

    let config_path = config.or_else(|| {
        let home = opencarrier_home();
        let candidate = home.join("config.toml");
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    });

    let kernel = match OpenCarrierKernel::boot(config_path.as_deref()) {
        Ok(k) => std::sync::Arc::new(k),
        Err(e) => {
            ui::error(&format!("Failed to boot kernel: {e}"));
            std::process::exit(1);
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            ui::error(&format!("Failed to create runtime: {e}"));
            std::process::exit(1);
        }
    };

    // Resolve agent ID (accept UUID or name)
    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => match kernel.registry.find_by_name(agent_id_str) {
            Some(entry) => {
                ui::hint(&format!("Using agent '{}' ({})", entry.name, entry.id));
                entry.id
            }
            None => {
                ui::error(&format!("Agent not found: {}", agent_id_str));
                std::process::exit(1);
            }
        },
    };

    // Build the full message from -m text + -f file contents
    let mut full_message = String::new();

    // Read file contents
    for (i, path) in files.iter().enumerate() {
        let content = if path == "-" {
            // Read from stdin
            let mut buf = String::new();
            match std::io::stdin().read_to_string(&mut buf) {
                Ok(_) => buf,
                Err(e) => {
                    ui::error(&format!("Failed to read stdin: {e}"));
                    std::process::exit(1);
                }
            }
        } else {
            match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    ui::error(&format!("Failed to read file '{}': {e}", path));
                    std::process::exit(1);
                }
            }
        };

        if i > 0 {
            full_message.push_str("\n\n");
        }
        let filename = if path == "-" {
            "stdin".to_string()
        } else {
            std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        };
        full_message.push_str(&format!("--- {} ---\n{}", filename, content));
    }

    // Append user message after file contents
    if !message.is_empty() {
        if !files.is_empty() {
            full_message.push_str("\n\n");
        }
        full_message.push_str(message);
    }

    if full_message.is_empty() {
        ui::error("No message or file content provided");
        std::process::exit(1);
    }

    ui::hint("Sending message...");
    match rt.block_on(kernel.send_message(agent_id, &full_message)) {
        Ok(result) => {
            if !result.silent {
                println!("{}", result.response);
            }
            if let Some(cost) = result.cost_usd {
                eprintln!("[cost: ${:.4}, {} iterations]", cost, result.iterations);
            }
        }
        Err(e) => {
            ui::error(&format!("Error: {e}"));
            std::process::exit(1);
        }
    }
}

fn cmd_agent_kill(config: Option<PathBuf>, agent_id_str: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{agent_id_str}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Agent {agent_id_str} killed.");
        } else {
            eprintln!(
                "Failed to kill agent: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let agent_id: AgentId = agent_id_str.parse().unwrap_or_else(|_| {
            eprintln!("Invalid agent ID: {agent_id_str}");
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.kill_agent(agent_id) {
            Ok(()) => println!("Agent {agent_id} killed."),
            Err(e) => {
                eprintln!("Failed to kill agent: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn cmd_agent_new(config: Option<PathBuf>, template_name: Option<String>) {
    let all_templates = templates::load_all_templates();
    if all_templates.is_empty() {
        ui::error_with_fix(
            "No agent templates found",
            "Run `opencarrier init` to set up the agents directory",
        );
        std::process::exit(1);
    }

    // Resolve template: by name or interactive picker
    let chosen = match template_name {
        Some(ref name) => match all_templates.iter().find(|t| t.name == *name) {
            Some(t) => t,
            None => {
                ui::error_with_fix(
                    &format!("Template '{name}' not found"),
                    "Run `opencarrier agent new` to see available templates",
                );
                std::process::exit(1);
            }
        },
        None => {
            ui::section("Available Agent Templates");
            ui::blank();
            for (i, t) in all_templates.iter().enumerate() {
                let desc = if t.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", t.description)
                };
                println!(
                    "    {:>2}. {:<22}{}",
                    i + 1,
                    t.name,
                    colored::Colorize::dimmed(desc.as_str())
                );
            }
            ui::blank();
            let choice = prompt_input("  Choose template [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(all_templates.len() - 1)
            };
            &all_templates[idx]
        }
    };

    // Spawn the agent
    spawn_template_agent(config, chosen);
}

/// Spawn an agent from a template, via daemon or in-process.
fn spawn_template_agent(config: Option<PathBuf>, template: &templates::AgentTemplate) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": template.content}))
                .send(),
        );
        if let Some(id) = body["agent_id"].as_str() {
            ui::blank();
            ui::success(&format!("Agent '{}' spawned", template.name));
            ui::kv("ID", id);
            if let Some(model) = body["model_name"].as_str() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                ui::kv("Model", &format!("{provider}/{model}"));
            }
            ui::blank();
            ui::hint(&format!("Chat: opencarrier chat {}", template.name));
        } else {
            ui::error(&format!(
                "Failed to spawn: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            ));
            std::process::exit(1);
        }
    } else {
        let manifest: AgentManifest = toml::from_str(&template.content).unwrap_or_else(|e| {
            ui::error_with_fix(
                &format!("Failed to parse template '{}': {e}", template.name),
                "The template manifest may be corrupted",
            );
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(manifest) {
            Ok(id) => {
                ui::blank();
                ui::success(&format!("Agent '{}' spawned (in-process)", template.name));
                ui::kv("ID", &id.to_string());
                ui::blank();
                ui::hint(&format!("Chat: opencarrier chat {}", template.name));
                ui::hint("Note: Agent will be lost when this process exits");
                ui::hint("For persistent agents, use `opencarrier start` first");
            }
            Err(e) => {
                ui::error(&format!("Failed to spawn agent: {e}"));
                std::process::exit(1);
            }
        }
    }
}

fn cmd_status(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        ui::section("OpenCarrier Daemon Status");
        ui::blank();
        ui::kv_ok("Status", body["status"].as_str().unwrap_or("?"));
        ui::kv(
            "Agents",
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv("Modality", body["default_modality"].as_str().unwrap_or("?"));
        ui::kv("Model", body["default_model"].as_str().unwrap_or("?"));
        ui::kv("API", &base);
        ui::kv("Dashboard", &format!("{base}/"));
        ui::kv("Data dir", body["data_dir"].as_str().unwrap_or("?"));
        ui::kv(
            "Uptime",
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );

        if let Some(agents) = body["agents"].as_array() {
            if !agents.is_empty() {
                ui::blank();
                ui::section("Active Agents");
                for a in agents {
                    println!(
                        "    {} ({}) -- {} [{}:{}]",
                        a["name"].as_str().unwrap_or("?"),
                        a["id"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["modality"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    );
                }
            }
        }
    } else {
        let kernel = boot_kernel(config);
        let agent_count = kernel.registry.count();

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "in-process",
                    "agent_count": agent_count,
                    "data_dir": kernel.config.data_dir.display().to_string(),
                    "default_provider": kernel.config.default_model.provider,
                    "default_model": kernel.config.default_model.model,
                    "daemon": false,
                }))
                .unwrap_or_default()
            );
            return;
        }

        ui::section("OpenCarrier Status (in-process)");
        ui::blank();
        ui::kv("Agents", &agent_count.to_string());
        ui::kv("Modality", &kernel.config.default_model.provider);
        ui::kv("Model", &kernel.config.default_model.model);
        ui::kv("Data dir", &kernel.config.data_dir.display().to_string());
        ui::kv_warn("Daemon", "NOT RUNNING");
        ui::blank();
        ui::hint("Run `opencarrier start` to launch the daemon");

        if agent_count > 0 {
            ui::blank();
            ui::section("Persisted Agents");
            for entry in kernel.registry.list() {
                println!("    {} ({}) -- {:?}", entry.name, entry.id, entry.state);
            }
        }
    }
}

fn cmd_doctor(json: bool, repair: bool) {
    let mut checks: Vec<serde_json::Value> = Vec::new();
    let mut all_ok = true;
    let mut repaired = false;

    if !json {
        ui::step("OpenCarrier Doctor");
        println!();
    }

    let home = dirs::home_dir();
    if let Some(_h) = &home {
        let opencarrier_dir = cli_opencarrier_home();

        // --- Check 1: OpenCarrier directory ---
        if opencarrier_dir.exists() {
            if !json {
                ui::check_ok(&format!(
                    "OpenCarrier directory: {}",
                    opencarrier_dir.display()
                ));
            }
            checks.push(serde_json::json!({"check": "opencarrier_dir", "status": "ok", "path": opencarrier_dir.display().to_string()}));
        } else if repair {
            if !json {
                ui::check_fail("OpenCarrier directory not found.");
            }
            let answer = prompt_input("    Create it now? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                if std::fs::create_dir_all(&opencarrier_dir).is_ok() {
                    restrict_dir_permissions(&opencarrier_dir);
                    for sub in ["data", "agents"] {
                        let _ = std::fs::create_dir_all(opencarrier_dir.join(sub));
                    }
                    if !json {
                        ui::check_ok("Created OpenCarrier directory");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create directory");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "opencarrier_dir", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("OpenCarrier directory not found. Run `opencarrier init` first.");
            }
            checks.push(serde_json::json!({"check": "opencarrier_dir", "status": "fail"}));
            all_ok = false;
        }

        // --- Check 2: .env file exists + permissions ---
        let env_path = opencarrier_dir.join(".env");
        if env_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&env_path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode == 0o600 {
                        if !json {
                            ui::check_ok(".env file (permissions OK)");
                        }
                    } else if repair {
                        let _ = std::fs::set_permissions(
                            &env_path,
                            std::fs::Permissions::from_mode(0o600),
                        );
                        if !json {
                            ui::check_ok(".env file (permissions fixed to 0600)");
                        }
                        repaired = true;
                    } else {
                        if !json {
                            ui::check_warn(&format!(
                                ".env file has loose permissions ({:o}), should be 0600",
                                mode
                            ));
                        }
                    }
                } else {
                    if !json {
                        ui::check_ok(".env file");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                if !json {
                    ui::check_ok(".env file");
                }
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "ok"}));
        } else {
            if !json {
                ui::check_warn(
                    ".env file not found (create with: opencarrier config set-key <provider>)",
                );
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "warn"}));
        }

        // --- Check 3: Config TOML syntax validation ---
        let config_path = opencarrier_dir.join("config.toml");
        if config_path.exists() {
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<toml::Value>(&config_content) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Config file: {}", config_path.display()));
                    }
                    checks.push(serde_json::json!({"check": "config_file", "status": "ok"}));
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config file has syntax errors: {e}"));
                        ui::hint("Fix with: opencarrier config edit");
                    }
                    checks.push(serde_json::json!({"check": "config_syntax", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        } else if repair {
            if !json {
                ui::check_fail("Config file not found.");
            }
            let answer = prompt_input("    Create default config? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                let (_provider, _api_key_env, _model) = detect_best_provider();
                let default_config = r#"# OpenCarrier Agent OS configuration
# See https://github.com/RightNow-AI/opencarrier for documentation

# For Docker, change to "0.0.0.0:4200" or set OPENCARRIER_LISTEN env var.
api_listen = "127.0.0.1:4200"

[brain]
config = "brain.json"

[memory]
decay_rate = 0.05
"#
                .to_string();
                let _ = std::fs::create_dir_all(&opencarrier_dir);
                if std::fs::write(&config_path, default_config).is_ok() {
                    restrict_file_permissions(&config_path);
                    if !json {
                        ui::check_ok("Created default config.toml");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create config.toml");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "config_file", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("Config file not found.");
            }
            checks.push(serde_json::json!({"check": "config_file", "status": "fail"}));
            all_ok = false;
        }

        // --- Check 4: Port availability ---
        // Read api_listen from config (default: 127.0.0.1:4200)
        let api_listen = {
            let cfg_path = opencarrier_dir.join("config.toml");
            if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|s| {
                        toml::from_str::<opencarrier_types::config::KernelConfig>(&s).ok()
                    })
                    .map(|c| c.api_listen)
                    .unwrap_or_else(|| "127.0.0.1:4200".to_string())
            } else {
                "127.0.0.1:4200".to_string()
            }
        };
        if !json {
            println!();
        }
        let daemon_running = find_daemon();
        if let Some(ref base) = daemon_running {
            if !json {
                ui::check_ok(&format!("Daemon running at {base}"));
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "ok", "url": base}));
        } else {
            if !json {
                ui::check_warn("Daemon not running (start with `opencarrier start`)");
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "warn"}));

            // Check if the configured port is available
            let bind_addr = if api_listen.starts_with("0.0.0.0") {
                api_listen.replacen("0.0.0.0", "127.0.0.1", 1)
            } else {
                api_listen.clone()
            };
            match std::net::TcpListener::bind(&bind_addr) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Port {api_listen} is available"));
                    }
                    checks.push(
                        serde_json::json!({"check": "port", "status": "ok", "address": api_listen}),
                    );
                }
                Err(_) => {
                    if !json {
                        ui::check_warn(&format!("Port {api_listen} is in use by another process"));
                    }
                    checks.push(serde_json::json!({"check": "port", "status": "warn", "address": api_listen}));
                }
            }
        }

        // --- Check 5: Stale daemon.json ---
        let daemon_json_path = opencarrier_dir.join("daemon.json");
        if daemon_json_path.exists() && daemon_running.is_none() {
            if repair {
                let _ = std::fs::remove_file(&daemon_json_path);
                if !json {
                    ui::check_ok("Removed stale daemon.json");
                }
                repaired = true;
            } else if !json {
                ui::check_warn(
                    "Stale daemon.json found (daemon not running). Run with --repair to clean up.",
                );
            }
            checks.push(serde_json::json!({"check": "stale_daemon_json", "status": if repair { "repaired" } else { "warn" }}));
        }

        // --- Check 6: Database file ---
        let db_path = opencarrier_dir.join("data").join("opencarrier.db");
        if db_path.exists() {
            // Quick SQLite magic bytes check
            if let Ok(bytes) = std::fs::read(&db_path) {
                if bytes.len() >= 16 && bytes.starts_with(b"SQLite format 3") {
                    if !json {
                        ui::check_ok("Database file (valid SQLite)");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "ok"}));
                } else {
                    if !json {
                        ui::check_fail("Database file exists but is not valid SQLite");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "fail"}));
                    all_ok = false;
                }
            }
        } else {
            if !json {
                ui::check_warn("No database file (will be created on first run)");
            }
            checks.push(serde_json::json!({"check": "database", "status": "warn"}));
        }

        // --- Check 7: Disk space ---
        #[cfg(unix)]
        {
            if let Ok(output) = std::process::Command::new("df")
                .args(["-m", &opencarrier_dir.display().to_string()])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Parse the available MB from df output (4th column of 2nd line)
                if let Some(line) = stdout.lines().nth(1) {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() >= 4 {
                        if let Ok(available_mb) = cols[3].parse::<u64>() {
                            if available_mb < 100 {
                                if !json {
                                    ui::check_warn(&format!(
                                        "Low disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "warn", "available_mb": available_mb}));
                            } else {
                                if !json {
                                    ui::check_ok(&format!(
                                        "Disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "ok", "available_mb": available_mb}));
                            }
                        }
                    }
                }
            }
        }

        // --- Check 8: Agent manifests parse correctly ---
        let agents_dir = opencarrier_dir.join("agents");
        if agents_dir.exists() {
            let mut agent_errors = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&agents_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Err(e) = toml::from_str::<AgentManifest>(&content) {
                                agent_errors.push((
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    e.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            if agent_errors.is_empty() {
                if !json {
                    ui::check_ok("Agent manifests are valid");
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "ok"}));
            } else {
                for (file, err) in &agent_errors {
                    if !json {
                        ui::check_fail(&format!("Invalid manifest {file}: {err}"));
                    }
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "fail", "errors": agent_errors.len()}));
                all_ok = false;
            }
        }
    } else {
        if !json {
            ui::check_fail("Could not determine home directory");
        }
        checks.push(serde_json::json!({"check": "home_dir", "status": "fail"}));
        all_ok = false;
    }

    // --- LLM providers ---
    if !json {
        println!("\n  LLM Providers:");
    }
    let provider_keys = [
        ("GROQ_API_KEY", "Groq", "groq"),
        ("OPENROUTER_API_KEY", "OpenRouter", "openrouter"),
        ("ANTHROPIC_API_KEY", "Anthropic", "anthropic"),
        ("OPENAI_API_KEY", "OpenAI", "openai"),
        ("DEEPSEEK_API_KEY", "DeepSeek", "deepseek"),
        ("GEMINI_API_KEY", "Gemini", "gemini"),
        ("GOOGLE_API_KEY", "Google", "google"),
        ("TOGETHER_API_KEY", "Together", "together"),
        ("MISTRAL_API_KEY", "Mistral", "mistral"),
        ("FIREWORKS_API_KEY", "Fireworks", "fireworks"),
    ];

    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // --- Check 9: Live key validation ---
            let valid = test_api_key(provider_id, env_var);
            if valid {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&format!("{name} ({env_var}) - key rejected (401/403)"));
            }
            any_key_set = true;
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": !valid}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    if !any_key_set {
        if !json {
            println!();
            ui::check_fail("No LLM provider API keys found!");
            ui::blank();
            ui::section("Getting an API key (free tiers)");
            ui::suggest_cmd("Groq:", "https://console.groq.com       (free, fast)");
            ui::suggest_cmd("Gemini:", "https://aistudio.google.com    (free tier)");
            ui::suggest_cmd("DeepSeek:", "https://platform.deepseek.com  (low cost)");
            ui::blank();
            ui::hint("Or run: opencarrier config set-key groq");
        }
        all_ok = false;
    }

    // --- Check 11: .env keys vs config api_key_env consistency ---
    {
        let opencarrier_dir = cli_opencarrier_home();
        let config_path = opencarrier_dir.join("config.toml");
        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
            // Look for api_key_env references in config
            for line in config_str.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("api_key_env") {
                    if let Some(val_part) = rest.strip_prefix('=') {
                        let val = val_part.trim().trim_matches('"');
                        if !val.is_empty() && std::env::var(val).is_err() {
                            if !json {
                                ui::check_warn(&format!(
                                    "Config references {val} but it is not set in env or .env"
                                ));
                            }
                            checks.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
                        }
                    }
                }
            }
        }
    }

    // --- Check 12: Config deserialization into KernelConfig ---
    {
        let opencarrier_dir = cli_opencarrier_home();
        let config_path = opencarrier_dir.join("config.toml");
        if config_path.exists() {
            if !json {
                println!("\n  Config Validation:");
            }
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<opencarrier_types::config::KernelConfig>(&config_content) {
                Ok(cfg) => {
                    if !json {
                        ui::check_ok("Config deserializes into KernelConfig");
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "ok"}));

                    // Check exec policy
                    let mode = format!("{:?}", cfg.exec_policy.mode);
                    let safe_bins_count = cfg.exec_policy.safe_bins.len();
                    if !json {
                        ui::check_ok(&format!(
                            "Exec policy: mode={mode}, safe_bins={safe_bins_count}"
                        ));
                    }
                    checks.push(serde_json::json!({"check": "exec_policy", "status": "ok", "mode": mode, "safe_bins": safe_bins_count}));

                    // Check includes
                    if !cfg.include.is_empty() {
                        let mut include_ok = true;
                        for inc in &cfg.include {
                            let inc_path = opencarrier_dir.join(inc);
                            if inc_path.exists() {
                                if !json {
                                    ui::check_ok(&format!("Include file: {inc}"));
                                }
                            } else if repair {
                                if !json {
                                    ui::check_warn(&format!("Include file missing: {inc}"));
                                }
                                include_ok = false;
                            } else {
                                if !json {
                                    ui::check_fail(&format!("Include file not found: {inc}"));
                                }
                                include_ok = false;
                                all_ok = false;
                            }
                        }
                        checks.push(serde_json::json!({"check": "config_includes", "status": if include_ok { "ok" } else { "fail" }, "count": cfg.include.len()}));
                    }

                    // Check MCP server configs
                    if !cfg.mcp_servers.is_empty() {
                        let mcp_count = cfg.mcp_servers.len();
                        if !json {
                            ui::check_ok(&format!("MCP servers configured: {mcp_count}"));
                        }
                        for server in &cfg.mcp_servers {
                            // Validate transport config
                            match &server.transport {
                                opencarrier_types::config::McpTransportEntry::Stdio {
                                    command,
                                    ..
                                } => {
                                    if command.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty command",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                opencarrier_types::config::McpTransportEntry::Sse { url } => {
                                    if url.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty URL",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                            }
                        }
                        checks.push(serde_json::json!({"check": "mcp_servers", "status": "ok", "count": mcp_count}));
                    }
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config fails KernelConfig deserialization: {e}"));
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        }
    }

    // --- Check 13: Skill registry health ---
    {
        if !json {
            println!("\n  Skills:");
        }
        let skills_dir = cli_opencarrier_home().join("skills");
        let mut skill_reg = opencarrier_skills::registry::SkillRegistry::new(skills_dir.clone());
        let _ = skill_reg.load_all();
        let skill_count = skill_reg.count();
        if !json {
            ui::check_ok(&format!("Skills loaded: {skill_count}"));
        }
        checks.push(
            serde_json::json!({"check": "skills", "status": "ok", "count": skill_count}),
        );

        // Check workspace skills if home dir available
        if skills_dir.exists() {
            match skill_reg.load_workspace_skills(&skills_dir) {
                Ok(_) => {
                    let total = skill_reg.count();
                    let ws_count = total.saturating_sub(skill_count);
                    if ws_count > 0 {
                        if !json {
                            ui::check_ok(&format!("Workspace skills loaded: {ws_count}"));
                        }
                        checks.push(serde_json::json!({"check": "workspace_skills", "status": "ok", "count": ws_count}));
                    }
                }
                Err(e) => {
                    if !json {
                        ui::check_warn(&format!("Failed to load workspace skills: {e}"));
                    }
                    checks.push(serde_json::json!({"check": "workspace_skills", "status": "warn", "error": e.to_string()}));
                }
            }
        }

        // Check for prompt injection issues in skill definitions
        // Only flag Critical-severity warnings (Warning-level hits are expected
        // in bundled skills that mention shell commands in educational context).
        let skills = skill_reg.list();
        let mut injection_warnings = 0;
        for skill in &skills {
            if let Some(ref prompt) = skill.manifest.prompt_context {
                let warnings =
                    opencarrier_skills::verify::SkillVerifier::scan_prompt_content(prompt);
                let has_critical = warnings.iter().any(|w| {
                    matches!(
                        w.severity,
                        opencarrier_skills::verify::WarningSeverity::Critical
                    )
                });
                if has_critical {
                    injection_warnings += 1;
                    if !json {
                        ui::check_warn(&format!(
                            "Prompt injection warning in skill: {}",
                            skill.manifest.skill.name
                        ));
                    }
                }
            }
        }
        if injection_warnings > 0 {
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "warn", "warnings": injection_warnings}));
        } else {
            if !json {
                ui::check_ok("All skills pass prompt injection scan");
            }
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "ok"}));
        }
    }

    // --- Check 15: Daemon health detail (if running) ---
    if let Some(ref base) = find_daemon() {
        if !json {
            println!("\n  Daemon Health:");
        }
        let client = daemon_client();
        match client.get(format!("{base}/api/health/detail")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
                        if !json {
                            ui::check_ok(&format!("Running agents: {agents}"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_agents", "status": "ok", "count": agents}));
                    }
                    if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                        let hours = uptime / 3600;
                        let mins = (uptime % 3600) / 60;
                        if !json {
                            ui::check_ok(&format!("Daemon uptime: {hours}h {mins}m"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_uptime", "status": "ok", "secs": uptime}));
                    }
                    if let Some(db_status) = body.get("database").and_then(|v| v.as_str()) {
                        if db_status == "connected" || db_status == "ok" {
                            if !json {
                                ui::check_ok("Database connectivity: OK");
                            }
                        } else {
                            if !json {
                                ui::check_fail(&format!("Database status: {db_status}"));
                            }
                            all_ok = false;
                        }
                        checks.push(serde_json::json!({"check": "daemon_db", "status": db_status}));
                    }
                }
            }
            Ok(resp) => {
                if !json {
                    ui::check_warn(&format!("Health detail returned {}", resp.status()));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn"}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&format!("Failed to query daemon health: {e}"));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check skills endpoint
        match client.get(format!("{base}/api/skills")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body.as_array() {
                        if !json {
                            ui::check_ok(&format!("Skills loaded in daemon: {}", arr.len()));
                        }
                        checks.push(serde_json::json!({"check": "daemon_skills", "status": "ok", "count": arr.len()}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP servers endpoint
        match client.get(format!("{base}/api/mcp/servers")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body.as_array() {
                        let connected = arr
                            .iter()
                            .filter(|s| {
                                s.get("connected")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                            })
                            .count();
                        if !json {
                            ui::check_ok(&format!(
                                "MCP servers: {} configured, {} connected",
                                arr.len(),
                                connected
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_mcp", "status": "ok", "configured": arr.len(), "connected": connected}));
                    }
                }
            }
            _ => {}
        }

        // Check extensions health endpoint
        match client.get(format!("{base}/api/integrations/health")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries = body.get("health").and_then(|h| h.as_array());
                    if let Some(arr) = entries {
                        let healthy = arr
                            .iter()
                            .filter(|v| {
                                v.get("status")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s.eq_ignore_ascii_case("ready"))
                                    .unwrap_or(false)
                            })
                            .count();
                        let total = arr.len();
                        if healthy == total {
                            if !json {
                                ui::check_ok(&format!(
                                    "Integration health: {healthy}/{total} healthy"
                                ));
                            }
                        } else if !json {
                            ui::check_warn(&format!(
                                "Integration health: {healthy}/{total} healthy"
                            ));
                        }
                        checks.push(serde_json::json!({"check": "integration_health", "status": if healthy == total { "ok" } else { "warn" }, "healthy": healthy, "total": total}));
                    }
                }
            }
            _ => {}
        }
    }

    if !json {
        println!();
    }
    match std::process::Command::new("rustc")
        .arg("--version")
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Rust: {version}"));
            }
            checks.push(serde_json::json!({"check": "rust", "status": "ok", "version": version}));
        }
        Err(_) => {
            if !json {
                ui::check_fail("Rust toolchain not found");
            }
            checks.push(serde_json::json!({"check": "rust", "status": "fail"}));
            all_ok = false;
        }
    }

    // Python runtime check
    match std::process::Command::new("python3")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Python: {version}"));
            }
            checks.push(serde_json::json!({"check": "python", "status": "ok", "version": version}));
        }
        _ => {
            // Try `python` instead
            match std::process::Command::new("python")
                .arg("--version")
                .output()
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !json {
                        ui::check_ok(&format!("Python: {version}"));
                    }
                    checks.push(
                        serde_json::json!({"check": "python", "status": "ok", "version": version}),
                    );
                }
                _ => {
                    if !json {
                        ui::check_warn("Python not found (needed for Python skill runtime)");
                    }
                    checks.push(serde_json::json!({"check": "python", "status": "warn"}));
                }
            }
        }
    }

    // Node.js runtime check
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Node.js: {version}"));
            }
            checks.push(serde_json::json!({"check": "node", "status": "ok", "version": version}));
        }
        _ => {
            if !json {
                ui::check_warn("Node.js not found (needed for Node skill runtime)");
            }
            checks.push(serde_json::json!({"check": "node", "status": "warn"}));
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "all_ok": all_ok,
                "checks": checks,
            }))
            .unwrap_or_default()
        );
    } else {
        println!();
        if all_ok {
            ui::success("All checks passed! OpenCarrier is ready.");
            ui::hint("Start the daemon: opencarrier start");
        } else if repaired {
            ui::success("Repairs applied. Re-run `opencarrier doctor` to verify.");
        } else {
            ui::error("Some checks failed.");
            if !repair {
                ui::hint("Run `opencarrier doctor --repair` to attempt auto-fix");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard command
// ---------------------------------------------------------------------------

fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint("No daemon running — starting one now...");
        match start_daemon_background() {
            Ok(url) => {
                ui::success("Daemon started");
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &format!("Could not start daemon: {e}"),
                    "Start it manually: opencarrier start",
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/");
    ui::success(&format!("Opening dashboard at {url}"));
    if copy_to_clipboard(&url) {
        ui::hint("URL copied to clipboard");
    }
    if !open_in_browser(&url) {
        ui::hint(&format!("Could not open browser. Visit: {url}"));
    }
}

/// Copy text to the system clipboard. Returns true on success.
fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to set clipboard (handles special characters better than cmd)
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard '{}'", text.replace('\'', "''")),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as IoWrite;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as IoWrite;
        // Try xclip first, then xsel
        let result = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        if result {
            return true;
        }
        std::process::Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

/// Try to open a URL in the default browser. Returns true on success.
pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        // Try multiple openers in order. xdg-open is the standard, but it
        // (or the browser it launches) can fail with EPERM in sandboxed
        // environments (containers, Snap, Flatpak, user-namespace
        // restrictions). Fall through to alternatives if any opener fails.
        let openers = [
            "xdg-open",
            "sensible-browser",
            "x-www-browser",
            "firefox",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ];
        for opener in &openers {
            let result = std::process::Command::new(opener)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if result.is_ok() {
                return true;
            }
        }
        false
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        false
    }
}

// ---------------------------------------------------------------------------
// Shell completion command
// ---------------------------------------------------------------------------

fn cmd_completion(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "opencarrier", &mut std::io::stdout());
}

/// Require a running daemon — exit with helpful message if not found.
fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &format!("`opencarrier {command}` requires a running daemon"),
            "Start the daemon: opencarrier start",
        );
        ui::hint("Or try `opencarrier chat` which works without a daemon");
        std::process::exit(1);
    })
}

fn boot_kernel(config: Option<PathBuf>) -> OpenCarrierKernel {
    match OpenCarrierKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Skill commands
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Provider / API key helpers
// ---------------------------------------------------------------------------

/// Map a provider name to its conventional environment variable name.
fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
///
/// Returns true if the key is accepted (status != 401/403).
/// Returns true on timeout/network errors (best-effort — don't block setup).
pub(crate) fn test_api_key(provider: &str, env_var: &str) -> bool {
    let key = match std::env::var(env_var) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // can't build client — assume ok
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(&key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(&key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&key)
            .send(),
        _ => return true, // unknown provider — skip test
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true, // network error — don't block setup
    }
}

// ---------------------------------------------------------------------------
// Interactive Providers command
// ---------------------------------------------------------------------------

fn cmd_providers() {
    let home = opencarrier_home();
    let brain_path = home.join("brain.json");

    if !brain_path.exists() {
        ui::error("No brain.json found. Run `opencarrier start` first to generate one.");
        return;
    }

    let brain_content = std::fs::read_to_string(&brain_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read brain.json: {e}"));
        std::process::exit(1);
    });

    let brain: opencarrier_types::brain::BrainConfig =
        serde_json::from_str(&brain_content).unwrap_or_else(|e| {
            ui::error(&format!("Failed to parse brain.json: {e}"));
            std::process::exit(1);
        });

    // Collect unique providers referenced by endpoints, with endpoint count
    let mut provider_names: Vec<String> = brain
        .endpoints
        .values()
        .map(|e| e.provider.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    provider_names.sort();

    if provider_names.is_empty() {
        ui::error("No providers found in brain.json.");
        return;
    }

    // Load .env to check which keys are set
    dotenv::load_dotenv();

    loop {
        println!();
        println!("{}", "  Providers".bold().bright_cyan());
        println!("{}", "  ──────────────────────────────────────────────────".bright_black());

        let mut any_missing = false;
        for (i, provider) in provider_names.iter().enumerate() {
            let env_var = provider_to_env_var(provider);
            let key_set = std::env::var(&env_var).map(|v| !v.is_empty()).unwrap_or(false);

            // Also check brain provider config for providers that don't need keys
            let no_key_needed = brain
                .providers
                .get(provider)
                .map(|p| p.api_key_env.is_empty())
                .unwrap_or(false);

            let status = if no_key_needed {
                format!("{}", "🏠 Local".bright_blue())
            } else if key_set {
                format!("{}", "✅ Key set".bright_green())
            } else {
                any_missing = true;
                format!("{}", "❌ Key needed".bright_red())
            };

            // Count endpoints using this provider
            let ep_count = brain
                .endpoints
                .values()
                .filter(|e| e.provider == *provider)
                .count();

            println!(
                "  {} {} {} ({} endpoint{})",
                format!("[{}]", i + 1).bright_yellow(),
                format!("{:<14}", provider).bold(),
                status,
                ep_count,
                if ep_count != 1 { "s" } else { "" },
            );
        }

        if any_missing {
            println!(
                "{}",
                "\n  ⚠  Some providers need API keys. Select a number to set it."
                    .bright_yellow()
            );
        }

        println!();
        println!(
            "{}",
            "  Enter number to set key, 'q' to quit:".bright_black()
        );
        print!("  > ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();

        if input == "q" || input == "quit" || input == "exit" {
            break;
        }

        // Parse selection
        let idx: usize = match input.parse::<usize>() {
            Ok(n) if n >= 1 && n <= provider_names.len() => n - 1,
            _ => {
                // Also allow typing provider name directly
                if let Some(pos) = provider_names.iter().position(|p| p == input) {
                    pos
                } else {
                    println!("{}", "  Invalid selection.".bright_red());
                    continue;
                }
            }
        };

        let provider = &provider_names[idx];
        let no_key_needed = brain
            .providers
            .get(provider)
            .map(|p| p.api_key_env.is_empty())
            .unwrap_or(false);

        if no_key_needed {
            println!(
                "  {} doesn't need an API key (local provider).",
                provider.bright_cyan()
            );
            continue;
        }

        let env_var = provider_to_env_var(provider);
        let existing = std::env::var(&env_var).unwrap_or_default();
        if !existing.is_empty() {
            println!(
                "  {} already has a key set ({}...). Enter new key to replace, or press Enter to skip.",
                provider.bright_cyan(),
                &existing[..existing.len().min(8)]
            );
        }

        let key = prompt_input(&format!("  API key for {}: ", provider.bright_cyan()));
        if key.is_empty() {
            println!("  Skipped.");
            continue;
        }

        // Save
        match dotenv::save_env_key(&env_var, &key) {
            Ok(()) => {
                // Update in-memory env so status reflects immediately
                std::env::set_var(&env_var, &key);

                // Test
                print!("  Testing... ");
                io::stdout().flush().unwrap();
                if test_api_key(provider, &env_var) {
                    println!("{}", "✔ OK".bright_green());
                } else {
                    println!("{}", "⚠ could not verify (may still work)".bright_yellow());
                }

                ui::success(&format!("Saved key for {provider}"));
            }
            Err(e) => {
                ui::error(&format!("Failed to save key: {e}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Background daemon start
// ---------------------------------------------------------------------------

/// Spawn `opencarrier start` as a detached background process.
///
/// Polls for daemon health for up to 10 seconds. Returns the daemon URL on success.
pub(crate) fn start_daemon_background() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find executable: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    // Poll for daemon readiness
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(url) = find_daemon() {
            return Ok(url);
        }
    }

    Err("Daemon did not become ready within 10 seconds".to_string())
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

fn cmd_config_show() {
    let home = opencarrier_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found at: {}", config_path.display());
        println!("Run `opencarrier init` to create one.");
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!("Error reading config: {e}");
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

fn cmd_config_edit() {
    let home = opencarrier_home();
    let config_path = home.join("config.toml");

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("Editor exited with: {s}");
        }
        Err(e) => {
            eprintln!("Failed to open editor '{editor}': {e}");
            eprintln!("Set $EDITOR to your preferred editor.");
        }
    }
}

fn cmd_config_get(key: &str) {
    let home = opencarrier_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `opencarrier init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax, or run `opencarrier config edit`",
        );
        std::process::exit(1);
    });

    // Navigate dotted path
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&format!("Key not found: {key}"));
                std::process::exit(1);
            }
        }
    }

    // Print value
    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

fn cmd_config_set(key: &str, value: &str) {
    let home = opencarrier_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `opencarrier init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax first",
        );
        std::process::exit(1);
    });

    // Navigate to parent and set key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];

    // Validate: single-part keys must be known scalar fields, not sections.
    // Writing a section name as a scalar silently breaks config deserialization.
    if parts.len() == 1 {
        let known_scalars = [
            "home_dir",
            "data_dir",
            "log_level",
            "api_listen",
            "network_enabled",
            "api_key",
            "language",
            "max_cron_jobs",
            "usage_footer",
            "workspaces_dir",
        ];
        if !known_scalars.contains(&last_key) {
            ui::error_with_fix(
                &format!("'{last_key}' is a section, not a scalar"),
                &format!("Use dotted notation: {last_key}.field_name"),
            );
            std::process::exit(1);
        }
    }

    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });

    // Try to preserve type: if the existing value is an integer, parse as int, etc.
    let new_value = if let Some(existing) = tbl.get(last_key) {
        match existing {
            toml::Value::Integer(_) => value
                .parse::<u64>()
                .map(|v| toml::Value::Integer(v as i64))
                .or_else(|_| value.parse::<i64>().map(toml::Value::Integer))
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        }
    } else {
        // No existing value — infer type from the string content
        if let Ok(b) = value.parse::<bool>() {
            toml::Value::Boolean(b)
        } else if let Ok(i) = value.parse::<u64>() {
            toml::Value::Integer(i as i64)
        } else if let Ok(i) = value.parse::<i64>() {
            toml::Value::Integer(i)
        } else if let Ok(f) = value.parse::<f64>() {
            toml::Value::Float(f)
        } else {
            toml::Value::String(value.to_string())
        }
    };

    tbl.insert(last_key.to_string(), new_value);

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&format!("Failed to serialize config: {e}"));
        std::process::exit(1);
    });

    let _ = std::fs::copy(&config_path, config_path.with_extension("toml.bak"));

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write config: {e}"));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&format!("Set {key} = {value}"));
}

fn cmd_config_unset(key: &str) {
    let home = opencarrier_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `opencarrier init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax first",
        );
        std::process::exit(1);
    });

    // Navigate to parent table and remove the final key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });

    if tbl.remove(last_key).is_none() {
        ui::error(&format!("Key not found: {key}"));
        std::process::exit(1);
    }

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&format!("Failed to serialize config: {e}"));
        std::process::exit(1);
    });

    let _ = std::fs::copy(&config_path, config_path.with_extension("toml.bak"));

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write config: {e}"));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&format!("Removed key: {key}"));
}

// ---------------------------------------------------------------------------
// Quick chat (OpenCarrier alias)
// ---------------------------------------------------------------------------

fn cmd_quick_chat(config: Option<PathBuf>, agent: String) {
    tui::chat_runner::run_chat_tui(config, Some(agent));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn opencarrier_home() -> PathBuf {
    if let Ok(home) = std::env::var("OPENCARRIER_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".opencarrier")
}

fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

// ---------------------------------------------------------------------------
// New command handlers
// ---------------------------------------------------------------------------

fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = match provider_filter {
            Some(p) => format!("{base}/api/models?provider={p}"),
            None => format!("{base}/api/models"),
        };
        let body = daemon_json(client.get(&url).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.as_array() {
            if arr.is_empty() {
                println!("No models found.");
                return;
            }
            println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
            println!("{}", "-".repeat(80));
            for m in arr {
                println!(
                    "{:<40} {:<16} {:<8} {}",
                    m["id"].as_str().unwrap_or("?"),
                    m["provider"].as_str().unwrap_or("?"),
                    m["tier"].as_str().unwrap_or("?"),
                    m["context_window"].as_u64().unwrap_or(0),
                );
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        // Standalone: use ModelCatalog directly
        let catalog = opencarrier_runtime::model_catalog::ModelCatalog::new();
        let models = catalog.list_models();
        if json {
            let arr: Vec<serde_json::Value> = models
                .iter()
                .filter(|m| provider_filter.is_none_or(|p| m.provider == p))
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "tier": format!("{:?}", m.tier),
                        "context_window": m.context_window,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        if models.is_empty() {
            println!("No models in catalog.");
            return;
        }
        println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
        println!("{}", "-".repeat(80));
        for m in models {
            if let Some(p) = provider_filter {
                if m.provider != p {
                    continue;
                }
            }
            println!(
                "{:<40} {:<16} {:<8} {}",
                m.id,
                m.provider,
                format!("{:?}", m.tier),
                m.context_window,
            );
        }
    }
}

fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(obj) = body.as_object() {
            println!("{:<30} RESOLVES TO", "ALIAS");
            println!("{}", "-".repeat(60));
            for (alias, target) in obj {
                println!("{:<30} {}", alias, target.as_str().unwrap_or("?"));
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = opencarrier_runtime::model_catalog::ModelCatalog::new();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
            return;
        }
        println!("{:<30} RESOLVES TO", "ALIAS");
        println!("{}", "-".repeat(60));
        for (alias, target) in aliases {
            println!("{:<30} {}", alias, target);
        }
    }
}

fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();
    // Use the config set approach through the API
    let body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&serde_json::json!({"key": "default_model.model", "value": model}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed to set model: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Default model set to: {model}"));
    }
}

/// Interactive model picker — shows numbered list, accepts number or model ID.
fn pick_model() -> String {
    let catalog = opencarrier_runtime::model_catalog::ModelCatalog::new();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error("No models in catalog.");
        std::process::exit(1);
    }

    // Group by provider for display
    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&opencarrier_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section("Select a model");
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            println!("    {idx:>3}. {:<36} {:?}", m.id, m.tier);
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    loop {
        let input = prompt_input("  Enter number or model ID: ");
        if input.is_empty() {
            continue;
        }
        // Try as number first
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&format!("Number out of range (1-{})", numbered.len()));
            continue;
        }
        // Accept direct model ID if it exists in catalog
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        // Accept as alias
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        // Accept any string (user might know a model not in catalog)
        return input;
    }
}

fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        println!(
            "{:<38} {:<16} {:<20} {:<8} PROMPT",
            "ID", "AGENT", "SCHEDULE", "ENABLED"
        );
        println!("{}", "-".repeat(100));
        for j in arr {
            println!(
                "{:<38} {:<16} {:<20} {:<8} {}",
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["cron_expr"].as_str().unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                j["prompt"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_cron_create(agent: &str, spec: &str, prompt: &str, explicit_name: Option<&str>) {
    let base = require_daemon("cron create");
    let client = daemon_client();

    // Use explicit name if provided, otherwise derive from agent + prompt
    let name = if let Some(n) = explicit_name {
        n.to_string()
    } else {
        let short_prompt: String = prompt
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        format!(
            "{}-{}",
            agent,
            if short_prompt.is_empty() {
                "job"
            } else {
                &short_prompt
            }
        )
    };

    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "name": name,
                "schedule": {
                    "kind": "cron",
                    "expr": spec
                },
                "action": {
                    "kind": "agent_turn",
                    "message": prompt
                }
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&format!("Cron job created: {id}"));
    } else {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} deleted."));
    }
}

fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} {endpoint}d."));
    }
}

fn cmd_sessions(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let url = match agent {
        Some(a) => format!("{base}/api/sessions?agent={a}"),
        None => format!("{base}/api/sessions"),
    };
    let body = daemon_json(client.get(&url).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No sessions found.");
            return;
        }
        println!("{:<38} {:<16} {:<8} LAST ACTIVE", "ID", "AGENT", "MSGS");
        println!("{}", "-".repeat(80));
        for s in arr {
            println!(
                "{:<38} {:<16} {:<8} {}",
                s["id"].as_str().unwrap_or("?"),
                s["agent_name"].as_str().unwrap_or("?"),
                s["message_count"].as_u64().unwrap_or(0),
                s["last_active"].as_str().unwrap_or("?"),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_logs(lines: usize, follow: bool) {
    let log_path = cli_opencarrier_home().join("tui.log");

    if !log_path.exists() {
        ui::error_with_fix(
            "Log file not found",
            &format!("Expected at: {}", log_path.display()),
        );
        std::process::exit(1);
    }

    if follow {
        // Use tail -f equivalent
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("tail")
                .args(["-f", "-n", &lines.to_string()])
                .arg(&log_path)
                .status();
        }
        #[cfg(windows)]
        {
            // On Windows, read in a loop
            let content = std::fs::read_to_string(&log_path).unwrap_or_default();
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            println!("--- Following {} (Ctrl+C to stop) ---", log_path.display());
            let mut last_len = content.len();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(new_content) = std::fs::read_to_string(&log_path) {
                    if new_content.len() > last_len {
                        print!("{}", &new_content[last_len..]);
                        last_len = new_content.len();
                    }
                }
            }
        }
    } else {
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{line}");
        }
    }
}


fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section("Security Status");
    ui::blank();
    ui::kv("Audit trail", "Merkle hash chain (SHA-256)");
    ui::kv("Taint tracking", "Information flow labels");
    ui::kv("WASM sandbox", "Dual metering (fuel + epoch)");
    ui::kv("Wire protocol", "OFP HMAC-SHA256 mutual auth");
    ui::kv("API keys", "Zeroizing<String> (auto-wipe on drop)");
    ui::kv("Manifests", "Ed25519 signed");
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv("Active agents", &agents.to_string());
    }
}

fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        println!("{:<24} {:<16} {:<12} EVENT", "TIMESTAMP", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for entry in arr {
            println!(
                "{:<24} {:<16} {:<12} {}",
                entry["timestamp"].as_str().unwrap_or("?"),
                entry["agent_name"].as_str().unwrap_or("?"),
                entry["event_type"].as_str().unwrap_or("?"),
                entry["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success("Audit trail integrity verified (Merkle chain valid).");
    } else {
        ui::error("Audit trail integrity check FAILED.");
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        println!("{:<30} VALUE", "KEY");
        println!("{}", "-".repeat(60));
        for kv in arr {
            println!(
                "{:<30} {}",
                kv["key"].as_str().unwrap_or("?"),
                kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Set {key} for agent '{agent}'."));
    }
}

fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Deleted key '{key}' for agent '{agent}'."));
    }
}

fn cmd_message(agent: &str, text: &str, json: bool) {
    let base = require_daemon("message");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent}/message"))
            .json(&serde_json::json!({"message": text}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_reset(confirm: bool) {
    let opencarrier_dir = cli_opencarrier_home();

    if !opencarrier_dir.exists() {
        println!(
            "Nothing to reset — {} does not exist.",
            opencarrier_dir.display()
        );
        return;
    }

    if !confirm {
        println!(
            "  This will delete all data in {}",
            opencarrier_dir.display()
        );
        println!("  Including: config, database, agent manifests, credentials.");
        println!();
        let answer = prompt_input("  Are you sure? Type 'yes' to confirm: ");
        if answer.trim() != "yes" {
            println!("  Cancelled.");
            return;
        }
    }

    match std::fs::remove_dir_all(&opencarrier_dir) {
        Ok(()) => ui::success(&format!("Removed {}", opencarrier_dir.display())),
        Err(e) => {
            ui::error(&format!(
                "Failed to remove {}: {e}",
                opencarrier_dir.display()
            ));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

async fn cmd_hub(cmd: HubCommands) {
    let config = opencarrier_kernel::config::load_config(None::<&std::path::Path>);

    let hub_url = if config.hub.url.is_empty() {
        eprintln!("Hub URL 未配置。请在 config.toml 中设置 [hub] url");
        std::process::exit(1);
    } else {
        config.hub.url.clone()
    };

    let api_key = match opencarrier_clone::hub::read_api_key(&config.hub.api_key_env) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    match cmd {
        HubCommands::Search { query } => {
            let q = query.unwrap_or_default();
            match opencarrier_clone::hub::search(&hub_url, &api_key, &q).await {
                Ok(output) => println!("{output}"),
                Err(e) => eprintln!("搜索失败: {e}"),
            }
        }
        HubCommands::Install { name, version } => {
            let home = cli_opencarrier_home();
            let device_id = match opencarrier_clone::hub::get_or_create_device_id(&home) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("生成 Device ID 失败: {e}");
                    std::process::exit(1);
                }
            };
            let workspace_dir = config.effective_workspaces_dir().join(&name);
            if workspace_dir.exists() {
                eprintln!("分身 '{}' 已存在: {}", name, workspace_dir.display());
                std::process::exit(1);
            }
            match opencarrier_clone::hub::install(
                &hub_url,
                &api_key,
                &name,
                version.as_deref(),
                &workspace_dir,
                &device_id,
            )
            .await
            {
                Ok(clone_name) => {
                    println!("分身 '{}' 已安装到: {}", clone_name, workspace_dir.display());
                    println!("运行 'opencarrier agent spawn {}' 启动分身", clone_name);
                }
                Err(e) => eprintln!("安装失败: {e}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(confirm: bool, keep_config: bool) {
    let opencarrier_dir = cli_opencarrier_home();
    let exe_path = std::env::current_exe().ok();

    // Step 1: Show what will be removed
    println!();
    println!(
        "  {}",
        "This will completely uninstall OpenCarrier from your system."
            .bold()
            .red()
    );
    println!();
    if opencarrier_dir.exists() {
        if keep_config {
            println!(
                "  • Remove data in {} (keeping config files)",
                opencarrier_dir.display()
            );
        } else {
            println!("  • Remove {}", opencarrier_dir.display());
        }
    }
    if let Some(ref exe) = exe_path {
        println!("  • Remove binary: {}", exe.display());
    }
    // Check cargo bin path
    let cargo_bin = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(if cfg!(windows) {
            "opencarrier.exe"
        } else {
            "opencarrier"
        });
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        println!("  • Remove cargo binary: {}", cargo_bin.display());
    }
    println!("  • Remove auto-start entries (if any)");
    println!("  • Clean PATH from shell configs (if any)");
    println!();

    // Step 2: Confirm
    if !confirm {
        let answer = prompt_input("  Type 'uninstall' to confirm: ");
        if answer.trim() != "uninstall" {
            println!("  Cancelled.");
            return;
        }
        println!();
    }

    // Step 3: Stop running daemon
    if find_daemon().is_some() {
        println!("  Stopping running daemon...");
        cmd_stop();
        // Give it a moment
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Force kill if still alive
        if find_daemon().is_some() {
            if let Some(info) = read_daemon_info(&opencarrier_dir) {
                force_kill_pid(info.pid);
                let _ = std::fs::remove_file(opencarrier_dir.join("daemon.json"));
            }
        }
    }

    // Step 4: Remove auto-start entries
    let user_home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    remove_autostart_entries(&user_home);

    // Step 5: Clean PATH from shell configs
    if let Some(ref exe) = exe_path {
        if let Some(bin_dir) = exe.parent() {
            clean_path_entries(&user_home, &bin_dir.to_string_lossy());
        }
    }

    // Step 6: Remove ~/.opencarrier/ data
    if opencarrier_dir.exists() {
        if keep_config {
            remove_dir_except_config(&opencarrier_dir);
            ui::success("Removed data (kept config files)");
        } else {
            match std::fs::remove_dir_all(&opencarrier_dir) {
                Ok(()) => ui::success(&format!("Removed {}", opencarrier_dir.display())),
                Err(e) => ui::error(&format!(
                    "Failed to remove {}: {e}",
                    opencarrier_dir.display()
                )),
            }
        }
    }

    // Step 7: Remove cargo bin copy if it exists and is separate from current exe
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        match std::fs::remove_file(&cargo_bin) {
            Ok(()) => ui::success(&format!("Removed {}", cargo_bin.display())),
            Err(e) => ui::error(&format!("Failed to remove {}: {e}", cargo_bin.display())),
        }
    }

    // Step 8: Remove the binary itself (must be last)
    if let Some(exe) = exe_path {
        remove_self_binary(&exe);
    }

    println!();
    ui::success("OpenCarrier has been uninstalled. Goodbye!");
}

/// Remove auto-start / launch-agent / systemd entries.
#[allow(unused_variables)]
fn remove_autostart_entries(home: &std::path::Path) {
    #[cfg(windows)]
    {
        // Windows: remove from HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "OpenCarrier",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Removed Windows auto-start registry entry");
            }
            _ => {} // Entry didn't exist — that's fine
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/ai.opencarrier.desktop.plist");
        if plist.exists() {
            // Unload first
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist) {
                Ok(()) => ui::success("Removed macOS launch agent"),
                Err(e) => ui::error(&format!("Failed to remove launch agent: {e}")),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_file = home.join(".config/autostart/OpenCarrier.desktop");
        if desktop_file.exists() {
            match std::fs::remove_file(&desktop_file) {
                Ok(()) => ui::success("Removed Linux autostart entry"),
                Err(e) => ui::error(&format!("Failed to remove autostart entry: {e}")),
            }
        }

        // Also check for systemd user service
        let service_file = home.join(".config/systemd/user/opencarrier.service");
        if service_file.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "opencarrier.service"])
                .output();
            match std::fs::remove_file(&service_file) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success("Removed systemd user service");
                }
                Err(e) => ui::error(&format!("Failed to remove systemd service: {e}")),
            }
        }
    }
}

/// Remove lines from shell config files that add opencarrier to PATH.
#[allow(unused_variables)]
fn clean_path_entries(home: &std::path::Path, opencarrier_dir: &str) {
    #[cfg(not(windows))]
    {
        let shell_files = [
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zshrc"),
            home.join(".config/fish/config.fish"),
        ];

        for path in &shell_files {
            if !path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| !is_opencarrier_path_line(line, opencarrier_dir))
                .collect();
            if filtered.len() < content.lines().count() {
                let new_content = filtered.join("\n");
                // Preserve trailing newline if original had one
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };
                if std::fs::write(path, &new_content).is_ok() {
                    ui::success(&format!("Cleaned PATH from {}", path.display()));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // Read User PATH via PowerShell, filter out opencarrier entries, write back
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH', 'User')",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let current = String::from_utf8_lossy(&out.stdout);
                let current = current.trim();
                if !current.is_empty() {
                    let dir_lower = opencarrier_dir.to_lowercase();
                    let filtered: Vec<&str> = current
                        .split(';')
                        .filter(|entry| {
                            let e = entry.trim().to_lowercase();
                            !e.is_empty() && !e.contains("opencarrier") && !e.contains(&dir_lower)
                        })
                        .collect();
                    if filtered.len() < current.split(';').count() {
                        let new_path = filtered.join(";");
                        let ps_cmd = format!(
                            "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
                            new_path.replace('\'', "''")
                        );
                        let result = std::process::Command::new("powershell")
                            .args(["-NoProfile", "-Command", &ps_cmd])
                            .output();
                        if result.is_ok_and(|o| o.status.success()) {
                            ui::success("Cleaned PATH from Windows user environment");
                        }
                    }
                }
            }
        }
    }
}

/// Returns true if a shell config line is an opencarrier PATH export.
/// Must match BOTH an opencarrier reference AND a PATH-setting pattern.
#[cfg(any(not(windows), test))]
fn is_opencarrier_path_line(line: &str, opencarrier_dir: &str) -> bool {
    let lower = line.to_lowercase();
    let has_opencarrier =
        lower.contains("opencarrier") || lower.contains(&opencarrier_dir.to_lowercase());
    if !has_opencarrier {
        return false;
    }
    // Match common PATH-setting patterns
    lower.contains("export path=")
        || lower.contains("export path =")
        || lower.starts_with("path=")
        || lower.contains("set -gx path")
        || lower.contains("fish_add_path")
}

/// Remove everything in ~/.opencarrier/ except config files.
fn remove_dir_except_config(opencarrier_dir: &std::path::Path) {
    let keep = ["config.toml", ".env", "secrets.env"];
    let Ok(entries) = std::fs::read_dir(opencarrier_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if keep.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remove the currently-running binary.
fn remove_self_binary(exe_path: &std::path::Path) {
    #[cfg(unix)]
    {
        // On Unix, running binaries can be unlinked — the OS keeps the inode
        // alive until the process exits.
        match std::fs::remove_file(exe_path) {
            Ok(()) => ui::success(&format!("Removed {}", exe_path.display())),
            Err(e) => ui::error(&format!(
                "Failed to remove binary {}: {e}",
                exe_path.display()
            )),
        }
    }

    #[cfg(windows)]
    {
        // Windows locks running executables. Rename first, then spawn a
        // detached process that waits briefly and deletes the renamed file.
        let old_path = exe_path.with_extension("exe.old");
        if std::fs::rename(exe_path, &old_path).is_err() {
            ui::error(&format!(
                "Could not rename binary for deferred deletion: {}",
                exe_path.display()
            ));
            return;
        }

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let del_cmd = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            old_path.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &del_cmd])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn();

        ui::success(&format!(
            "Removed {} (deferred cleanup)",
            exe_path.display()
        ));
    }
}

#[cfg(test)]
mod tests {

    // --- Doctor command unit tests ---

    #[test]
    fn test_doctor_config_deser_default() {
        // Default KernelConfig should serialize/deserialize round-trip
        let config = opencarrier_types::config::KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: opencarrier_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.api_listen, config.api_listen);
    }

    #[test]
    fn test_doctor_config_include_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"
include = ["providers.toml", "agents.toml"]

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: opencarrier_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.include.len(), 2);
        assert_eq!(config.include[0], "providers.toml");
        assert_eq!(config.include[1], "agents.toml");
    }

    #[test]
    fn test_doctor_exec_policy_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"

[exec_policy]
mode = "allowlist"
safe_bins = ["ls", "cat", "echo"]
timeout_secs = 30

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: opencarrier_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.exec_policy.mode,
            opencarrier_types::config::ExecSecurityMode::Allowlist
        );
        assert_eq!(config.exec_policy.safe_bins.len(), 3);
        assert_eq!(config.exec_policy.timeout_secs, 30);
    }

    #[test]
    fn test_doctor_mcp_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "github"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#;
        let config: opencarrier_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "github");
        match &config.mcp_servers[0].transport {
            opencarrier_types::config::McpTransportEntry::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_doctor_skill_injection_scan_clean() {
        let clean_content = "This is a normal skill prompt with helpful instructions.";
        let warnings =
            opencarrier_skills::verify::SkillVerifier::scan_prompt_content(clean_content);
        assert!(warnings.is_empty(), "Clean content should have no warnings");
    }

    #[test]
    fn test_doctor_hook_event_variants() {
        // Verify all 4 hook event types are constructable
        use opencarrier_types::agent::HookEvent;
        let events = [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
            HookEvent::AgentLoopEnd,
        ];
        assert_eq!(events.len(), 4);
    }

    // --- Uninstall command unit tests ---

    #[test]
    fn test_uninstall_path_line_filter() {
        use super::is_opencarrier_path_line;
        let dir = "/home/user/.opencarrier/bin";

        // Should match: opencarrier PATH exports
        assert!(is_opencarrier_path_line(
            r#"export PATH="$HOME/.opencarrier/bin:$PATH""#,
            dir
        ));
        assert!(is_opencarrier_path_line(
            r#"export PATH="/home/user/.opencarrier/bin:$PATH""#,
            dir
        ));
        assert!(is_opencarrier_path_line(
            "set -gx PATH $HOME/.opencarrier/bin $PATH",
            dir
        ));
        assert!(is_opencarrier_path_line(
            "fish_add_path $HOME/.opencarrier/bin",
            dir
        ));

        // Should NOT match: unrelated PATH exports
        assert!(!is_opencarrier_path_line(
            r#"export PATH="$HOME/.cargo/bin:$PATH""#,
            dir
        ));
        assert!(!is_opencarrier_path_line(
            r#"export PATH="/usr/local/bin:$PATH""#,
            dir
        ));

        // Should NOT match: opencarrier lines that aren't PATH-related
        assert!(!is_opencarrier_path_line("# opencarrier config", dir));
        assert!(!is_opencarrier_path_line("alias of=opencarrier", dir));
    }
}
