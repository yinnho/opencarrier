//! JSON-RPC 2.0 client for MCP tool calls.

use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Send a JSON-RPC 2.0 `tools/call` request to an MCP endpoint.
pub async fn call_tool(
    http: &Client,
    url: &str,
    tool_name: &str,
    arguments: &Value,
    timeout_ms: Option<u64>,
) -> Result<String, String> {
    let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

    let id = format!(
        "mcp_{}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        hex::encode(&rand::random::<[u8; 4]>())
    );

    let request_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let response = http
        .post(url)
        .header("Accept", "application/json")
        .header(
            "User-Agent",
            format!("OpenCarrier/{}", env!("CARGO_PKG_VERSION")),
        )
        .json(&request_body)
        .timeout(Duration::from_millis(timeout))
        .send()
        .await
        .map_err(|e| format!("MCP HTTP error: {e}"))?;

    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("MCP_AUTH_EXPIRED".to_string());
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("MCP response parse error: {e}"))?;

    // Check for JSON-RPC error
    if let Some(error) = body.get("error") {
        let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("MCP error {code}: {msg}"));
    }

    // Extract result content
    let result = body
        .get("result")
        .ok_or_else(|| format!("MCP response missing result: {}", truncate_json(&body, 200)))?;

    // Check isError flag
    if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
        let text = extract_text_content(result);
        return Err(format!("MCP tool error: {text}"));
    }

    Ok(extract_text_content(result))
}

/// Extract text from MCP result content array.
fn extract_text_content(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        let texts: Vec<String> = content
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }

    // Fallback: serialize the whole result
    serde_json::to_string(result).unwrap_or_else(|_| format!("{result}"))
}

fn truncate_json(v: &Value, max_len: usize) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.len() > max_len {
        format!("{}...", &s[..max_len])
    } else {
        s
    }
}
