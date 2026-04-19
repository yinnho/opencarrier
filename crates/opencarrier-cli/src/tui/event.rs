//! Event system: crossterm polling, tick timer, streaming bridges.

use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_runtime::agent_loop::AgentLoopResult;
use opencarrier_runtime::llm_driver::StreamEvent;
use opencarrier_types::agent::AgentId;
use ratatui::crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind};
use std::sync::{mpsc, Arc};
use std::time::Duration;

// ── AppEvent ────────────────────────────────────────────────────────────────

/// Unified application event.
pub enum AppEvent {
    /// A crossterm key press event (filtered to Press only).
    Key(KeyEvent),
    /// Periodic tick for animations (spinners, etc.).
    Tick,
    /// A streaming event from the LLM (daemon SSE or kernel mpsc).
    Stream(StreamEvent),
    /// The streaming agent loop finished.
    StreamDone(Result<AgentLoopResult, String>),
    /// The kernel finished booting in the background.
    KernelReady(Arc<OpenCarrierKernel>),
    /// The kernel failed to boot.
    KernelError(String),
    /// An agent was successfully spawned (daemon mode).
    AgentSpawned { id: String, name: String },
    /// Agent spawn failed.
    AgentSpawnError(String),
}

/// Spawn the crossterm polling + tick thread. Returns sender + receiver.
pub fn spawn_event_thread(
    tick_rate: Duration,
) -> (mpsc::Sender<AppEvent>, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    let poll_tx = tx.clone();

    std::thread::spawn(move || {
        loop {
            if event::poll(tick_rate).unwrap_or(false) {
                if let Ok(ev) = event::read() {
                    let sent = match ev {
                        // CRITICAL: only forward Press events — Windows sends
                        // Release and Repeat too, which causes double/triple input
                        CtEvent::Key(key) if key.kind == KeyEventKind::Press => {
                            poll_tx.send(AppEvent::Key(key))
                        }
                        _ => Ok(()),
                    };
                    if sent.is_err() {
                        break;
                    }
                }
            } else {
                // No event within tick_rate → send tick for spinner animations
                if poll_tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    (tx, rx)
}

/// Spawn a background thread that boots the kernel.
pub fn spawn_kernel_boot(config: Option<std::path::PathBuf>, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        // Create a tokio runtime context so any tokio::spawn calls during
        // boot (e.g. publish_event via set_self_handle) find the reactor.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        match OpenCarrierKernel::boot(config.as_deref()) {
            Ok(k) => {
                let k = Arc::new(k);
                k.set_self_handle();
                let _ = tx.send(AppEvent::KernelReady(k));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::KernelError(format!("{e}")));
            }
        }
    });
}

/// Spawn a background thread for in-process streaming.
pub fn spawn_inprocess_stream(
    kernel: Arc<OpenCarrierKernel>,
    agent_id: AgentId,
    message: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!("Runtime error: {e}"))));
                return;
            }
        };

        // Enter the runtime context so tokio::spawn inside
        // send_message_streaming() finds the reactor.
        let _guard = rt.enter();

        match kernel.send_message_streaming(agent_id, &message, None, None, None) {
            Ok((mut rx, handle)) => {
                rt.block_on(async {
                    while let Some(ev) = rx.recv().await {
                        if tx.send(AppEvent::Stream(ev)).is_err() {
                            return;
                        }
                    }
                    let result = handle
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()));
                    let _ = tx.send(AppEvent::StreamDone(result));
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!("{e}"))));
            }
        }
    });
}

/// Spawn a background thread for daemon SSE streaming.
pub fn spawn_daemon_stream(
    base_url: String,
    agent_id: String,
    message: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read};

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap();

        let url = format!("{base_url}/api/agents/{agent_id}/message/stream");
        let resp = client
            .post(&url)
            .json(&serde_json::json!({"message": message}))
            .send();

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(_) => {
                let fallback = daemon_fallback(&base_url, &agent_id, &message);
                let _ = tx.send(AppEvent::StreamDone(fallback));
                return;
            }
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!("Connection failed: {e}"))));
                return;
            }
        };

        struct RespReader(reqwest::blocking::Response);
        impl Read for RespReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }

        // Accumulate usage across all iterations (tool-use loops send
        // multiple ContentComplete events — one per LLM call).  Do NOT
        // return early on "done": true — the SSE stream continues until
        // the server closes the connection after the agent loop finishes.
        let mut total_input_tokens: u64 = 0;
        let mut total_output_tokens: u64 = 0;

        let reader = BufReader::new(RespReader(resp));
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.is_empty() || line.starts_with("event:") {
                continue;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = json.get("content").and_then(|c| c.as_str()) {
                        let _ = tx.send(AppEvent::Stream(StreamEvent::TextDelta {
                            text: content.to_string(),
                        }));
                    }
                    if let Some(tool) = json.get("tool").and_then(|t| t.as_str()) {
                        if json.get("input").is_none() {
                            let _ = tx.send(AppEvent::Stream(StreamEvent::ToolUseStart {
                                id: String::new(),
                                name: tool.to_string(),
                            }));
                        } else {
                            let _ = tx.send(AppEvent::Stream(StreamEvent::ToolUseEnd {
                                id: String::new(),
                                name: tool.to_string(),
                                input: json["input"].clone(),
                            }));
                        }
                    }
                    if json.get("done").and_then(|d| d.as_bool()) == Some(true) {
                        let usage = json.get("usage").cloned().unwrap_or_default();
                        total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        // Forward as ContentComplete so the UI can update
                        // token display, but do NOT terminate — the agent
                        // loop may continue with tool results.
                        let _ = tx.send(AppEvent::Stream(StreamEvent::ContentComplete {
                            stop_reason: opencarrier_types::message::StopReason::EndTurn,
                            usage: opencarrier_types::message::TokenUsage {
                                input_tokens: total_input_tokens,
                                output_tokens: total_output_tokens,
                            },
                        }));
                    }
                }
            }
        }

        // Connection closed — agent loop is truly done.
        let _ = tx.send(AppEvent::StreamDone(Ok(AgentLoopResult {
            response: String::new(),
            total_usage: opencarrier_types::message::TokenUsage {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
            },
            iterations: 0,
            silent: false,
            directives: Default::default(),
        })));
    });
}

/// Blocking fallback for daemon chat (non-streaming).
fn daemon_fallback(
    base_url: &str,
    agent_id: &str,
    message: &str,
) -> Result<AgentLoopResult, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(format!("{base_url}/api/agents/{agent_id}/message"))
        .json(&serde_json::json!({"message": message}))
        .send()
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;

    if let Some(response) = body.get("response").and_then(|r| r.as_str()) {
        let input_tokens = body["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = body["output_tokens"].as_u64().unwrap_or(0);
        Ok(AgentLoopResult {
            response: response.to_string(),
            total_usage: opencarrier_types::message::TokenUsage {
                input_tokens,
                output_tokens,
            },
            iterations: body["iterations"].as_u64().unwrap_or(0) as u32,
            silent: false,
            directives: Default::default(),
        })
    } else {
        Err(body["error"]
            .as_str()
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// Spawn a background thread that spawns an agent on the daemon.
pub fn spawn_daemon_agent(base_url: String, toml_content: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        let resp = client
            .post(format!("{base_url}/api/agents"))
            .json(&serde_json::json!({"manifest_toml": toml_content}))
            .send();

        match resp {
            Ok(r) => {
                let body: serde_json::Value = r.json().unwrap_or_default();
                if let Some(id) = body.get("agent_id").and_then(|v| v.as_str()) {
                    let name = body["name"].as_str().unwrap_or("agent").to_string();
                    let _ = tx.send(AppEvent::AgentSpawned {
                        id: id.to_string(),
                        name,
                    });
                } else {
                    let _ = tx.send(AppEvent::AgentSpawnError(
                        body["error"]
                            .as_str()
                            .unwrap_or("Failed to spawn agent")
                            .to_string(),
                    ));
                }
            }
            Err(e) => {
                let _ = tx.send(AppEvent::AgentSpawnError(format!("{e}")));
            }
        }
    });
}
