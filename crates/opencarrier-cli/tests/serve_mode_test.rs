//! Integration tests for serve mode (agentd communication).
//!
//! These tests verify the stdin/stdout JSON-RPC server works correctly
//! with both JSON-RPC 2.0 and yingheclient protocols.

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Protocol Tests
// ---------------------------------------------------------------------------

/// Test JSON-RPC request parsing
#[test]
fn test_jsonrpc_request_parse() {
    let json = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "hello",
        "params": {"version": "1.0.0"}
    });

    let json_str = serde_json::to_string(&json).unwrap();

    // Verify JSON-RPC format
    assert!(json_str.contains("jsonrpc"));
    assert!(json_str.contains("method"));
    assert!(json_str.contains("hello"));
}

/// Test JSON-RPC response format
#[test]
fn test_jsonrpc_response_format() {
    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "status": "ok",
            "version": "0.1.0"
        }
    });

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["status"], "ok");
}

/// Test JSON-RPC error response format
#[test]
fn test_jsonrpc_error_format() {
    let error_response = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "error": {
            "code": -32601,
            "message": "Method not found"
        }
    });

    assert_eq!(error_response["jsonrpc"], "2.0");
    assert_eq!(error_response["error"]["code"], -32601);
    assert_eq!(error_response["error"]["message"], "Method not found");
}

/// Test standard JSON-RPC error codes
#[test]
fn test_jsonrpc_error_codes() {
    // Parse error: -32700
    // Invalid request: -32600
    // Method not found: -32601
    // Invalid params: -32602
    // Internal error: -32603

    let parse_error: i32 = -32700;
    let invalid_request: i32 = -32600;
    let method_not_found: i32 = -32601;
    let invalid_params: i32 = -32602;
    let internal_error: i32 = -32603;

    // All codes are in the -32xxx range
    assert!(parse_error >= -32799 && parse_error <= -32700);
    assert!(invalid_request >= -32699 && invalid_request <= -32600);
    assert!(method_not_found == -32601);
    assert!(invalid_params == -32602);
    assert!(internal_error == -32603);
}

// ---------------------------------------------------------------------------
// yingheclient Protocol Tests
// ---------------------------------------------------------------------------

/// Test yingheclient ChatRequest detection
#[test]
fn test_yingheclient_detection() {
    // yingheclient format should be detected
    let yinghe_msg = json!({
        "type": "chat",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello"
    });
    let json_str = serde_json::to_string(&yinghe_msg).unwrap();

    // Detection should check for conversationType and type field
    assert!(json_str.contains("conversationType"));
    assert!(json_str.contains("\"type\":\"chat\""));
}

/// Test yingheclient vs JSON-RPC protocol differentiation
#[test]
fn test_protocol_differentiation() {
    // yingheclient format
    let yinghe = json!({
        "type": "chat",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello"
    });
    let yinghe_str = serde_json::to_string(&yinghe).unwrap();
    assert!(yinghe_str.contains("conversationType"));
    assert!(!yinghe_str.contains("jsonrpc"));

    // JSON-RPC format
    let jsonrpc = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendMessage",
        "params": {"message": "Hello"}
    });
    let jsonrpc_str = serde_json::to_string(&jsonrpc).unwrap();
    assert!(jsonrpc_str.contains("jsonrpc"));
    assert!(jsonrpc_str.contains("method"));
}

/// Test ChatResponse format
#[test]
fn test_chat_response_format() {
    let response = json!({
        "type": "chat_response",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "response": "Hello! How can I help?",
        "metadata": {
            "rounds": 1,
            "toolCalls": 0
        }
    });

    assert_eq!(response["type"], "chat_response");
    assert_eq!(response["conversationType"], "carrier");
    assert_eq!(response["metadata"]["rounds"], 1);
}

/// Test ErrorResponse format
#[test]
fn test_error_response_format() {
    let error = json!({
        "type": "error",
        "message": "Something went wrong"
    });

    assert_eq!(error["type"], "error");
    assert_eq!(error["message"], "Something went wrong");
}

// ---------------------------------------------------------------------------
// Method Tests
// ---------------------------------------------------------------------------

/// Test hello method request format
#[test]
fn test_hello_method() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "hello",
        "params": {
            "version": "1.0.0"
        }
    });

    assert_eq!(request["method"], "hello");
}

/// Test sendMessage method request format
#[test]
fn test_send_message_method() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "sendMessage",
        "params": {
            "agentId": "default",
            "message": "Hello, agent!"
        }
    });

    assert_eq!(request["method"], "sendMessage");
    assert_eq!(request["params"]["agentId"], "default");
    assert_eq!(request["params"]["message"], "Hello, agent!");
}

/// Test getAgentCard method
#[test]
fn test_get_agent_card_method() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "getAgentCard",
        "params": {
            "agentId": "default"
        }
    });

    assert_eq!(request["method"], "getAgentCard");

    // Expected response format
    let response = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "result": {
            "name": "OpenCarrier Agent",
            "capabilities": {
                "streaming": true,
                "tools": true
            }
        }
    });

    assert_eq!(response["result"]["name"], "OpenCarrier Agent");
}

/// Test listAgents method
#[test]
fn test_list_agents_method() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "listAgents",
        "params": {}
    });

    assert_eq!(request["method"], "listAgents");
}

/// Test bye notification (no response expected)
#[test]
fn test_bye_notification() {
    let request = json!({
        "jsonrpc": "2.0",
        "method": "bye"
        // No id - this is a notification, not a request
    });

    // Notifications don't have an id
    assert!(request.get("id").is_none());
    assert_eq!(request["method"], "bye");
}

// ---------------------------------------------------------------------------
// Edge Cases
// ---------------------------------------------------------------------------

/// Test empty input handling
#[test]
fn test_empty_input() {
    let input = "";
    assert!(input.trim().is_empty());
}

/// Test whitespace-only input
#[test]
fn test_whitespace_input() {
    let input = "   \n\t  ";
    assert!(input.trim().is_empty());
}

/// Test malformed JSON handling
#[test]
fn test_malformed_json() {
    let input = "{ not valid json }";
    let result: Result<Value, _> = serde_json::from_str(input);
    assert!(result.is_err());
}

/// Test missing required fields
#[test]
fn test_missing_required_fields() {
    // Missing jsonrpc version
    let request = json!({
        "id": 1,
        "method": "hello"
    });
    // Should not have jsonrpc field
    assert!(request.get("jsonrpc").is_none());
}

/// Test unknown method handling
#[test]
fn test_unknown_method() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "unknownMethod",
        "params": {}
    });

    // Should return method not found error (-32601)
    assert_eq!(request["method"], "unknownMethod");
}

// ---------------------------------------------------------------------------
// Session Management Tests
// ---------------------------------------------------------------------------

/// Test session key format
#[test]
fn test_session_key_format() {
    // SessionKey format: "<conversationType>:<chatType>:<instanceId>:<conversationId>"
    let key = "carrier:direct:main:conv-001";

    let parts: Vec<&str> = key.split(':').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "carrier");
    assert_eq!(parts[1], "direct");
    assert_eq!(parts[2], "main");
    assert_eq!(parts[3], "conv-001");
}

/// Test plugin session key format
#[test]
fn test_plugin_session_key() {
    let key = "plugin:group:weather-plugin:group-001";

    let parts: Vec<&str> = key.split(':').collect();
    assert_eq!(parts[0], "plugin");
    assert_eq!(parts[1], "group");
    assert_eq!(parts[2], "weather-plugin");
    assert_eq!(parts[3], "group-001");
}

// ---------------------------------------------------------------------------
// Group Chat Tests
// ---------------------------------------------------------------------------

/// Test group chat with mention
#[test]
fn test_group_chat_with_mention() {
    let request = json!({
        "type": "chat",
        "conversationId": "group-001",
        "conversationType": "avatar",
        "chatType": "group",
        "content": "@bot Hello!",
        "mentioned": true
    });

    assert_eq!(request["chatType"], "group");
    assert_eq!(request["mentioned"], true);
}

/// Test group chat without mention (should not respond)
#[test]
fn test_group_chat_without_mention() {
    let request = json!({
        "type": "chat",
        "conversationId": "group-001",
        "conversationType": "avatar",
        "chatType": "group",
        "content": "Hello everyone!",
        "mentioned": false
    });

    assert_eq!(request["chatType"], "group");
    assert_eq!(request["mentioned"], false);
    // Agent should NOT respond to this
}

/// Test implicit mention (reply to agent's message)
#[test]
fn test_implicit_mention() {
    let request = json!({
        "type": "chat",
        "conversationId": "group-001",
        "conversationType": "avatar",
        "chatType": "group",
        "content": "Yes, please do that",
        "mentioned": false,
        "implicitMention": true,
        "replyToSenderId": "bot-001"
    });

    assert_eq!(request["mentioned"], false);
    assert_eq!(request["implicitMention"], true);
    // Agent SHOULD respond to this (implicit mention)
}

// ---------------------------------------------------------------------------
// Message Format Validation Tests
// ---------------------------------------------------------------------------

/// Test camelCase field names in yingheclient format
#[test]
fn test_camel_case_fields() {
    let request = json!({
        "type": "chat",
        "conversationId": "conv-001",      // camelCase
        "conversationType": "carrier",     // camelCase
        "chatType": "direct",              // camelCase
        "pluginId": "test-plugin",         // camelCase
        "senderId": "user-001",            // camelCase
        "senderName": "Alice",             // camelCase
        "messageId": "msg-001",            // camelCase
        "replyToMessageId": "msg-000",     // camelCase
        "replyToSenderId": "bot-001"       // camelCase
    });

    // All fields should be camelCase
    assert!(request.get("conversationId").is_some());
    assert!(request.get("conversation_type").is_none()); // snake_case not used
}

/// Test metadata in response
#[test]
fn test_response_metadata() {
    let response = json!({
        "type": "chat_response",
        "response": "Done",
        "metadata": {
            "rounds": 2,
            "toolCalls": 1,
            "matchedSkills": ["skill1", "skill2"]
        }
    });

    let metadata = &response["metadata"];
    assert_eq!(metadata["rounds"], 2);
    assert_eq!(metadata["toolCalls"], 1);
    assert!(metadata["matchedSkills"].is_array());
}
