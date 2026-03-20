//! A2A Serve Mode — stdin/stdout JSON-RPC server for agentd.
//!
//! This module implements the serve mode for agentd托管模式.
//!
//! Protocol: JSON-RPC 2.0 over newline-delimited stdin/stdout.
//! - Each line from stdin is a JSON-RPC request
//! - Each line to stdout is a JSON-RPC response
//! - All logs go to stderr (NEVER pollute stdout)
//!
//! Methods:
//! - hello: Client handshake
//! - sendMessage: Send message to agent
//! - getAgentCard: Get agent capabilities
//! - listAgents: List all agents
//! - compactMemory: Compress memory (App initiates, Carrier executes)
//! - bye: Close connection (notification, no response)

use opencarrier_kernel::OpenCarrierKernel;
use opencarrier_runtime::a2a::{AgentCapabilities, AgentCard, AgentSkill};
use opencarrier_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tracing::{debug, error, info};

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
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

// ---------------------------------------------------------------------------
// Serve Mode Entry Point
// ---------------------------------------------------------------------------

/// Run the serve mode: read from stdin, process, write to stdout.
pub fn run_serve_mode(config_path: Option<std::path::PathBuf>) {
    // All logs to stderr
    eprintln!("[serve] Starting opencarrier serve mode");

    // Boot kernel
    let kernel = match OpenCarrierKernel::boot(config_path.as_deref()) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("[serve] Failed to boot kernel: {e}");
            std::process::exit(1);
        }
    };

    // Create tokio runtime for async operations
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[serve] Failed to create runtime: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("[serve] Kernel booted, ready for requests");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    // Main request loop
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF
                debug!("[serve] EOF received, exiting");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                debug!("[serve] Received: {}", &trimmed[..trimmed.len().min(200)]);

                // Parse and handle request
                let response = match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                    Ok(req) => handle_request(&kernel, &rt, &req),
                    Err(e) => {
                        // Parse error
                        error!("[serve] Parse error: {e}");
                        Some(jsonrpc_error(None, PARSE_ERROR, &format!("Parse error: {e}")))
                    }
                };

                // Write response
                if let Some(resp) = response {
                    let resp_json = match resp {
                        Response::Success(r) => serde_json::to_string(&r),
                        Response::Error(r) => serde_json::to_string(&r),
                    };

                    match resp_json {
                        Ok(json) => {
                            if let Err(e) = writeln!(writer, "{}", json) {
                                error!("[serve] Write error: {e}");
                                break;
                            }
                            if let Err(e) = writer.flush() {
                                error!("[serve] Flush error: {e}");
                                break;
                            }
                        }
                        Err(e) => {
                            error!("[serve] Serialize error: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                error!("[serve] Read error: {e}");
                break;
            }
        }
    }

    info!("[serve] Serve mode exiting");
}

/// Response type (success or error)
enum Response {
    Success(JsonRpcResponse),
    Error(JsonRpcErrorResponse),
}

// ---------------------------------------------------------------------------
// Request Handling
// ---------------------------------------------------------------------------

/// Handle a JSON-RPC request and return an optional response.
fn handle_request(
    kernel: &Arc<OpenCarrierKernel>,
    rt: &tokio::runtime::Runtime,
    req: &JsonRpcRequest,
) -> Option<Response> {
    // Validate jsonrpc version
    if req.jsonrpc != "2.0" {
        return Some(jsonrpc_error(req.id.clone(), INVALID_REQUEST, "Invalid jsonrpc version"));
    }

    match req.method.as_str() {
        "hello" => handle_hello(req),
        "sendMessage" => handle_send_message(kernel, rt, req),
        "getAgentCard" => handle_get_agent_card(kernel, req),
        "listAgents" => handle_list_agents(kernel, req),
        "compactMemory" => handle_compact_memory(kernel, rt, req),
        "bye" => {
            // Notification, no response
            info!("[serve] Received bye, connection closing");
            None
        }
        _ => Some(jsonrpc_error(req.id.clone(), METHOD_NOT_FOUND, &format!("Method not found: {}", req.method))),
    }
}

/// Handle hello handshake
fn handle_hello(req: &JsonRpcRequest) -> Option<Response> {
    let params = req.params.as_ref().and_then(|p| p.as_object());

    let client_name = params
        .and_then(|p| p.get("clientName"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let client_version = params
        .and_then(|p| p.get("clientVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0");

    info!("[serve] Hello from {} v{}", client_name, client_version);

    Some(jsonrpc_success(
        req.id.clone(),
        json!({
            "status": "ok",
            "serverName": "opencarrier",
            "serverVersion": env!("CARGO_PKG_VERSION")
        }),
    ))
}

/// Handle sendMessage method
fn handle_send_message(
    kernel: &Arc<OpenCarrierKernel>,
    rt: &tokio::runtime::Runtime,
    req: &JsonRpcRequest,
) -> Option<Response> {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, "Missing params")),
    };

    let agent_id_str = params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let message = match params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, "Missing message")),
    };

    // Parse agent ID
    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try to find agent by name
            match kernel.registry.find_by_name(agent_id_str) {
                Some(entry) => entry.id,
                None => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, &format!("Agent not found: {}", agent_id_str))),
            }
        }
    };

    // Send message
    match rt.block_on(kernel.send_message(agent_id, message)) {
        Ok(result) => Some(jsonrpc_success(
            req.id.clone(),
            json!({
                "response": result.response,
                "iterations": result.iterations,
                "costUsd": result.cost_usd
            }),
        )),
        Err(e) => {
            error!("[serve] send_message error: {e}");
            Some(jsonrpc_error(req.id.clone(), INTERNAL_ERROR, &format!("Error: {e}")))
        }
    }
}

/// Handle getAgentCard method
fn handle_get_agent_card(kernel: &Arc<OpenCarrierKernel>, req: &JsonRpcRequest) -> Option<Response> {
    let params = req.params.as_ref().and_then(|p| p.as_object());

    let agent_id_str = params
        .and_then(|p| p.get("agentId"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    // Try to get agent manifest
    let card = if let Ok(agent_id) = agent_id_str.parse::<AgentId>() {
        if let Some(entry) = kernel.registry.get(agent_id) {
            Some(build_agent_card_from_entry(&entry))
        } else {
            None
        }
    } else {
        // Find by name (blocking, need runtime)
        None
    };

    match card {
        Some(c) => Some(jsonrpc_success(req.id.clone(), serde_json::to_value(c).unwrap_or_default())),
        None => Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, &format!("Agent not found: {}", agent_id_str))),
    }
}

/// Handle listAgents method
fn handle_list_agents(kernel: &Arc<OpenCarrierKernel>, req: &JsonRpcRequest) -> Option<Response> {
    let agents: Vec<Value> = kernel
        .registry
        .list()
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id.to_string(),
                "name": entry.name,
                "description": entry.manifest.description,
                "tools": entry.manifest.capabilities.tools
            })
        })
        .collect();

    Some(jsonrpc_success(req.id.clone(), json!({ "agents": agents })))
}

/// Handle compactMemory method
///
/// App 发起记忆压缩，Carrier 执行压缩。
/// App 发送需要压缩的消息列表，Carrier 使用 LLM 生成摘要。
fn handle_compact_memory(
    kernel: &Arc<OpenCarrierKernel>,
    rt: &tokio::runtime::Runtime,
    req: &JsonRpcRequest,
) -> Option<Response> {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, "Missing params")),
    };

    // Get messages to compact
    let messages = match params.get("messages") {
        Some(Value::Array(arr)) => arr,
        _ => return Some(jsonrpc_error(req.id.clone(), INVALID_PARAMS, "Missing or invalid messages array")),
    };

    // Get keep recent count
    let keep_recent = params
        .get("keepRecent")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    if messages.is_empty() {
        return Some(jsonrpc_success(req.id.clone(), json!({
            "summary": "",
            "recentMessages": [],
            "compactedCount": 0
        })));
    }

    // Split messages: older ones to compact, recent ones to keep
    let split_point = messages.len().saturating_sub(keep_recent);
    let to_compact = &messages[..split_point];
    let recent = &messages[split_point..];

    // Use LLM to generate summary
    let summary_prompt = format!(
        "请用简洁的中文总结以下对话的关键信息（包括用户意图、重要决策、待办事项等）：\n\n{}",
        serde_json::to_string_pretty(to_compact).unwrap_or_default()
    );

    // Get default agent for summarization
    let default_agent = kernel
        .registry
        .list()
        .first()
        .map(|e| e.id)
        .unwrap_or_default();

    match rt.block_on(kernel.send_message(default_agent, &summary_prompt)) {
        Ok(result) => Some(jsonrpc_success(
            req.id.clone(),
            json!({
                "summary": result.response,
                "recentMessages": recent,
                "compactedCount": to_compact.len()
            }),
        )),
        Err(e) => {
            error!("[serve] compactMemory error: {e}");
            Some(jsonrpc_error(req.id.clone(), INTERNAL_ERROR, &format!("Compaction failed: {e}")))
        }
    }
}

/// Build agent card from registry entry
fn build_agent_card_from_entry(entry: &opencarrier_types::agent::AgentEntry) -> AgentCard {
    let skills: Vec<AgentSkill> = entry
        .manifest
        .capabilities
        .tools
        .iter()
        .map(|tool| AgentSkill {
            id: tool.clone(),
            name: tool.replace('_', " "),
            description: format!("Can use the {} tool", tool),
            tags: vec!["tool".to_string()],
            examples: vec![],
        })
        .collect();

    AgentCard {
        name: entry.name.clone(),
        description: entry.manifest.description.clone(),
        url: "a2a://localhost".to_string(),
        version: "0.1.0".to_string(),
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        skills,
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC Response Helpers
// ---------------------------------------------------------------------------

fn jsonrpc_success(id: Option<Value>, result: Value) -> Response {
    Response::Success(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result,
    })
}

fn jsonrpc_error(id: Option<Value>, code: i32, message: &str) -> Response {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_parsing() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"clientName":"test"}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(Value::Number(1.into())));
        assert_eq!(req.method, "hello");
    }

    #[test]
    fn test_jsonrpc_request_without_params() {
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"listAgents"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "listAgents");
        assert!(req.params.is_none());
    }

    #[test]
    fn test_jsonrpc_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"bye"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "bye");
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
