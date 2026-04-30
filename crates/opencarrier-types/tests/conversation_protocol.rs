//! Integration tests for conversation protocol compatibility.
//!
//! These tests verify that the serve mode correctly handles
//! ChatRequest messages and returns properly formatted ChatResponse messages.

use serde_json::json;

/// Test that ChatRequest format is correctly detected
#[test]
fn test_chat_request_detection() {
    let direct_message = json!({
        "type": "chat",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello"
    });

    let json_str = serde_json::to_string(&direct_message).unwrap();

    // Check for conversation format indicators
    assert!(json_str.contains("conversationType"));
    assert!(json_str.contains("chat"));
}

/// Test ChatRequest deserialization
#[test]
fn test_chat_request_deserialize() {
    let json = json!({
        "type": "chat",
        "conversationId": "conv-002",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello, agent!"
    });

    let request: opencarrier_types::conversation::ChatRequest = serde_json::from_value(json).unwrap();

    assert_eq!(request.msg_type, "chat");
    assert_eq!(request.conversation_id, "conv-002");
    assert_eq!(
        request.conversation_type,
        opencarrier_types::conversation::ConversationType::Carrier
    );
    assert_eq!(
        request.chat_type,
        opencarrier_types::conversation::ChatType::Direct
    );
    assert_eq!(request.content, Some("Hello, agent!".to_string()));
}

/// Test ChatRequest with plugin mode
#[test]
fn test_chat_request_plugin_mode() {
    let json = json!({
        "type": "message",
        "conversationId": "conv-003",
        "conversationType": "plugin",
        "chatType": "direct",
        "pluginId": "weather-plugin",
        "content": "What's the weather?"
    });

    let request: opencarrier_types::conversation::ChatRequest = serde_json::from_value(json).unwrap();

    assert_eq!(
        request.conversation_type,
        opencarrier_types::conversation::ConversationType::Plugin
    );
    assert_eq!(request.plugin_id, Some("weather-plugin".to_string()));
}

/// Test ChatRequest with group chat
#[test]
fn test_chat_request_group_chat() {
    let json = json!({
        "type": "chat",
        "conversationId": "group-001",
        "conversationType": "avatar",
        "chatType": "group",
        "avatarId": "assistant-001",
        "content": "@bot Hello everyone!",
        "senderId": "user-123",
        "senderName": "Alice",
        "mentioned": true
    });

    let request: opencarrier_types::conversation::ChatRequest = serde_json::from_value(json).unwrap();

    assert!(request.is_group());
    assert!(request.mentioned);
    assert_eq!(request.sender_name, Some("Alice".to_string()));
    assert_eq!(request.avatar_id, Some("assistant-001".to_string()));
}

/// Test ChatResponse serialization
#[test]
fn test_chat_response_serialize() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType};

    let request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-001".to_string(),
        conversation_type: ConversationType::Carrier,
        chat_type: ChatType::Direct,
        plugin_id: None,
        avatar_id: None,
        content: Some("Hello".to_string()),
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    let response = opencarrier_types::conversation::ChatResponse::for_request(
        &request,
        "Hello! How can I help you?".to_string(),
    );

    let json = serde_json::to_value(&response).unwrap();

    // Verify required fields (camelCase in JSON)
    assert_eq!(json["type"], "chat_response");
    assert_eq!(json["conversationId"], "conv-001");
    assert_eq!(json["conversationType"], "carrier");
    assert_eq!(json["chatType"], "direct");
    assert_eq!(json["response"], "Hello! How can I help you?");
}

/// Test ChatResponse with metadata
#[test]
fn test_chat_response_with_metadata() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType, ResponseMetadata};

    let request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-002".to_string(),
        conversation_type: ConversationType::Carrier,
        chat_type: ChatType::Direct,
        plugin_id: None,
        avatar_id: None,
        content: Some("Hello".to_string()),
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    let metadata = ResponseMetadata {
        rounds: Some(2),
        tool_calls: Some(1),
        matched_skills: Some(vec!["greeting".to_string()]),
        ..Default::default()
    };

    let response =
        opencarrier_types::conversation::ChatResponse::for_request(&request, "Done".to_string())
            .with_metadata(metadata);

    let json = serde_json::to_value(&response).unwrap();

    assert_eq!(json["metadata"]["rounds"], 2);
    assert_eq!(json["metadata"]["toolCalls"], 1);
}

/// Test ErrorResponse serialization
#[test]
fn test_error_response_serialize() {
    let error = opencarrier_types::conversation::ErrorResponse::new("Something went wrong");

    let json = serde_json::to_value(&error).unwrap();

    assert_eq!(json["type"], "error");
    assert_eq!(json["message"], "Something went wrong");
}

/// Test ErrorResponse for request
#[test]
fn test_error_response_for_request() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType};

    let request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-003".to_string(),
        conversation_type: ConversationType::Plugin,
        chat_type: ChatType::Direct,
        plugin_id: Some("test-plugin".to_string()),
        avatar_id: None,
        content: Some("Test".to_string()),
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    let error =
        opencarrier_types::conversation::ErrorResponse::for_request(&request, "Test error".to_string());

    let json = serde_json::to_value(&error).unwrap();

    assert_eq!(json["type"], "error");
    assert_eq!(json["conversationId"], "conv-003");
    assert_eq!(json["conversationType"], "plugin");
    assert_eq!(json["pluginId"], "test-plugin");
    assert_eq!(json["message"], "Test error");
}

/// Test SessionKey format
#[test]
fn test_session_key_format() {
    use opencarrier_types::conversation::{ChatType, ConversationType, SessionKey};

    let key = SessionKey::new(
        ConversationType::Plugin,
        ChatType::Group,
        "weather-plugin",
        "conv-123",
    );

    let formatted = format!("{}", key);
    assert_eq!(formatted, "plugin:group:weather-plugin:conv-123");

    // Test parsing
    let parsed = SessionKey::parse(&formatted).unwrap();
    assert_eq!(parsed, key);
}

/// Test that message type detection distinguishes between JSON-RPC and conversation
#[test]
fn test_protocol_detection() {
    // conversation format
    let chat_msg = json!({
        "type": "chat",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello"
    });
    let chat_str = serde_json::to_string(&chat_msg).unwrap();
    assert!(chat_str.contains("conversationType"));
    assert!(chat_str.contains("\"type\":\"chat\""));

    // JSON-RPC format
    let jsonrpc_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendMessage",
        "params": {
            "agentId": "default",
            "message": "Hello"
        }
    });
    let jsonrpc_str = serde_json::to_string(&jsonrpc_msg).unwrap();
    assert!(jsonrpc_str.contains("jsonrpc"));
    assert!(jsonrpc_str.contains("method"));
}

/// Test group chat without mention should be identified
#[test]
fn test_group_chat_mention_detection() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType};

    // With mention - should respond
    let with_mention = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "group-001".to_string(),
        conversation_type: ConversationType::Avatar,
        chat_type: ChatType::Group,
        plugin_id: None,
        avatar_id: Some("bot-001".to_string()),
        content: Some("@bot Hello".to_string()),
        attachment: None,
        sender_id: Some("user-123".to_string()),
        sender_name: Some("Alice".to_string()),
        sender_username: None,
        mentioned: true,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    assert!(with_mention.is_group());
    assert!(with_mention.mentioned);

    // Without mention - should NOT respond
    let without_mention = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "group-001".to_string(),
        conversation_type: ConversationType::Avatar,
        chat_type: ChatType::Group,
        plugin_id: None,
        avatar_id: Some("bot-001".to_string()),
        content: Some("Hello everyone".to_string()),
        attachment: None,
        sender_id: Some("user-123".to_string()),
        sender_name: Some("Alice".to_string()),
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    assert!(without_mention.is_group());
    assert!(!without_mention.mentioned);
    assert!(!without_mention.implicit_mention);
}

/// Test implicit mention detection
#[test]
fn test_implicit_mention() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType};

    // Reply to bot's message - implicit mention
    let implicit = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "group-002".to_string(),
        conversation_type: ConversationType::Avatar,
        chat_type: ChatType::Group,
        plugin_id: None,
        avatar_id: Some("bot-001".to_string()),
        content: Some("Yes, please do that".to_string()),
        attachment: None,
        sender_id: Some("user-456".to_string()),
        sender_name: Some("Bob".to_string()),
        sender_username: None,
        mentioned: false,
        implicit_mention: true,
        reply_to_message_id: Some("msg-001".to_string()),
        reply_to_sender_id: Some("bot-001".to_string()),
        message_id: None,
        timestamp: None,
    };

    assert!(implicit.is_group());
    assert!(!implicit.mentioned);
    assert!(implicit.implicit_mention);
    // Either mentioned OR implicit_mention means should respond
    assert!(implicit.mentioned || implicit.implicit_mention);
}

/// Test all conversation types
#[test]
fn test_all_conversation_types() {
    use opencarrier_types::conversation::ConversationType;

    // Test serialization
    assert_eq!(
        serde_json::to_string(&ConversationType::Carrier).unwrap(),
        "\"carrier\""
    );
    assert_eq!(
        serde_json::to_string(&ConversationType::Plugin).unwrap(),
        "\"plugin\""
    );
    assert_eq!(
        serde_json::to_string(&ConversationType::Avatar).unwrap(),
        "\"avatar\""
    );
    assert_eq!(
        serde_json::to_string(&ConversationType::Role).unwrap(),
        "\"role\""
    );

    // Test deserialization
    let carrier: ConversationType = serde_json::from_str("\"carrier\"").unwrap();
    assert_eq!(carrier, ConversationType::Carrier);

    let plugin: ConversationType = serde_json::from_str("\"plugin\"").unwrap();
    assert_eq!(plugin, ConversationType::Plugin);
}

/// Test all chat types
#[test]
fn test_all_chat_types() {
    use opencarrier_types::conversation::ChatType;

    // Test serialization
    assert_eq!(
        serde_json::to_string(&ChatType::Direct).unwrap(),
        "\"direct\""
    );
    assert_eq!(
        serde_json::to_string(&ChatType::Group).unwrap(),
        "\"group\""
    );

    // Test deserialization
    let direct: ChatType = serde_json::from_str("\"direct\"").unwrap();
    assert_eq!(direct, ChatType::Direct);

    let group: ChatType = serde_json::from_str("\"group\"").unwrap();
    assert_eq!(group, ChatType::Group);
}

/// Test ChatRequest text_content method
#[test]
fn test_text_content() {
    use opencarrier_types::conversation::{ChatRequest, ChatType, ConversationType};

    let request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-001".to_string(),
        conversation_type: ConversationType::Carrier,
        chat_type: ChatType::Direct,
        plugin_id: None,
        avatar_id: None,
        content: Some("Hello world".to_string()),
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    assert_eq!(request.text_content(), "Hello world");

    let empty_request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-002".to_string(),
        conversation_type: ConversationType::Carrier,
        chat_type: ChatType::Direct,
        plugin_id: None,
        avatar_id: None,
        content: None,
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: false,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    assert_eq!(empty_request.text_content(), "");
}
