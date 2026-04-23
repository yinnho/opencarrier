//! ACP Mode — stdin/stdout JSON-RPC for ACP connectors (aginx, etc.).
//!
//! Protocol: ACP (Agent Client Protocol) over JSON-RPC 2.0, ndjson transport.
//! - Each line from stdin is a JSON-RPC request
//! - Each line to stdout is a JSON-RPC response or notification
//! - All logs go to stderr (NEVER pollute stdout)

use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_memory::acp_session::AcpSessionStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};

use crate::acp;

// ---------------------------------------------------------------------------
// JSON-RPC Types
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 Request
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 Response (success)
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub result: Value,
}

/// JSON-RPC 2.0 Error Response
#[derive(Debug, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub error: JsonRpcError,
}

/// JSON-RPC 2.0 Error Object
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// Standard JSON-RPC error codes
pub(crate) const PARSE_ERROR: i32 = -32700;
pub(crate) const INVALID_REQUEST: i32 = -32600;
pub(crate) const METHOD_NOT_FOUND: i32 = -32601;
pub(crate) const INVALID_PARAMS: i32 = -32602;
pub(crate) const INTERNAL_ERROR: i32 = -32603;

/// Shared writer type for concurrent stdout access.
pub(crate) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

// ---------------------------------------------------------------------------
// Serve Mode Entry Point
// ---------------------------------------------------------------------------

/// Run ACP mode: read from stdin, process, write to stdout.
///
/// Architecture:
/// - Stdin reader runs in a background thread, sending lines via channel
/// - Main loop dispatches requests to ACP handlers
/// - `session/prompt` spawns a worker thread (non-blocking, supports cancel)
/// - Stdout is shared via `Arc<Mutex<>>` for concurrent writes
pub fn run_acp_mode(config_path: Option<std::path::PathBuf>) {
    // All logs to stderr
    eprintln!("[acp] Starting opencarrier ACP mode");

    // Boot kernel
    let kernel = match OpenCarrierKernel::boot(config_path.as_deref()) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("[acp] Failed to boot kernel: {e}");
            std::process::exit(1);
        }
    };

    // Create tokio runtime for async operations
    let rt: Arc<tokio::runtime::Runtime> = Arc::new(
        match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("[acp] Failed to create runtime: {e}");
                std::process::exit(1);
            }
        },
    );

    // Initialize ACP session store for conversation persistence
    let acp_store = init_acp_session_store(&kernel);

    // Initialize ACP connection state
    let mut acp_state = acp::AcpConnectionState::default();

    eprintln!("[acp] Kernel booted, ready for requests");

    // Shared stdout writer — prompt threads and main loop both write here
    let writer: SharedWriter = Arc::new(Mutex::new(Box::new(io::stdout())));

    // Stdin reader thread — sends lines to main loop via channel
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = std::io::BufReader::new(stdin.lock());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if tx.send(trimmed).is_err() {
                        break; // Main loop exited
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Main dispatch loop
    while let Ok(line) = rx.recv() {
        debug!("[acp] Received: {}", &line[..line.len().min(200)]);

        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(req) => {
                if acp::is_acp_method(&req.method) {
                    acp::handle_acp_request(
                        &kernel,
                        &rt,
                        &req,
                        &mut acp_state,
                        &writer,
                        &acp_store,
                    )
                } else {
                    Some(jsonrpc_error(
                        req.id.clone(),
                        METHOD_NOT_FOUND,
                        &format!("Method not found: {}", req.method),
                    ))
                }
            }
            Err(e) => {
                error!("[acp] Parse error: {e}");
                Some(jsonrpc_error(
                    None,
                    PARSE_ERROR,
                    &format!("Parse error: {e}"),
                ))
            }
        };

        // Write response (if any — session/prompt returns None, writes its own)
        if let Some(resp) = response {
            write_response(&writer, resp);
        }
    }

    info!("[acp] ACP mode exiting");
}

/// Write a JSON-RPC response to the shared writer.
pub(crate) fn write_response(writer: &SharedWriter, resp: Response) {
    let json = match resp {
        Response::Success(r) => serde_json::to_string(&r),
        Response::Error(r) => serde_json::to_string(&r),
    };
    match json {
        Ok(j) => {
            let mut w = writer.lock().unwrap();
            let _ = writeln!(w, "{}", j);
            let _ = w.flush();
        }
        Err(e) => error!("[acp] Serialize error: {e}"),
    }
}

/// Response type (success or error)
pub(crate) enum Response {
    Success(JsonRpcResponse),
    Error(JsonRpcErrorResponse),
}

// ---------------------------------------------------------------------------
// JSON-RPC Response Helpers
// ---------------------------------------------------------------------------

pub(crate) fn jsonrpc_success(id: Option<Value>, result: Value) -> Response {
    Response::Success(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result,
    })
}

pub(crate) fn jsonrpc_error(id: Option<Value>, code: i32, message: &str) -> Response {
    Response::Error(JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        id,
        error: JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        },
    })
}

// ---------------------------------------------------------------------------
// Session Store Initialization
// ---------------------------------------------------------------------------

/// Initialize ACP session store.
///
/// Creates the sessions directory and returns a file-backed session manager.
fn init_acp_session_store(_kernel: &OpenCarrierKernel) -> AcpSessionStore {
    let sessions_dir = dirs::home_dir()
        .map(|h| h.join(".opencarrier").join("sessions"))
        .unwrap_or_else(|| std::path::PathBuf::from("./sessions"));

    if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
        error!("[acp] Failed to create sessions directory: {e}");
    }

    info!(
        "[acp] ACP session store initialized at {:?}",
        sessions_dir
    );
    AcpSessionStore::new(&sessions_dir)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_jsonrpc_request_parsing() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(Value::Number(1.into())));
        assert_eq!(req.method, "initialize");
    }

    #[test]
    fn test_jsonrpc_request_without_params() {
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"session/list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "session/list");
        assert!(req.params.is_none());
    }

    #[test]
    fn test_jsonrpc_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s1"}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "session/cancel");
        assert!(req.id.is_none()); // Notification has no id
    }

    #[test]
    fn test_jsonrpc_response_serialization() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            result: json!({"status": "ok"}),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"result\""));
    }

    #[test]
    fn test_jsonrpc_error_response() {
        let resp = JsonRpcErrorResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(2.into())),
            error: JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(json.contains("-32601"));
    }
}
