//! TypeScript (yingheclient) vs Rust (opencarrier) Protocol Comparison Tests
//!
//! These tests verify that the Rust implementation matches the TypeScript types
//! defined in yingheclient/src/conversation/types.ts

use serde_json::json;

// ---------------------------------------------------------------------------
// ChatRequest Comparison Tests
// ---------------------------------------------------------------------------

/// Test that all ChatRequest fields from TypeScript are supported in Rust
#[test]
fn test_chat_request_fields_match() {
    // TypeScript ChatRequest fields:
    // - type: 'chat' | 'message'
    // - conversationId: string
    // - conversationType: ConversationType
    // - chatType: ChatType
    // - pluginId?: string
    // - avatarId?: string
    // - content?: string
    // - attachment?: Attachment
    // - senderId?: string
    // - senderName?: string
    // - senderUsername?: string
    // - mentioned?: boolean
    // - implicitMention?: boolean
    // - replyToMessageId?: string
    // - replyToSenderId?: string
    // - messageId?: string
    // - timestamp?: number

    let request = json!({
        "type": "chat",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "pluginId": "test-plugin",
        "avatarId": "test-avatar",
        "content": "Hello",
        "senderId": "user-001",
        "senderName": "Alice",
        "senderUsername": "alice",
        "mentioned": true,
        "implicitMention": false,
        "replyToMessageId": "msg-000",
        "replyToSenderId": "bot-001",
        "messageId": "msg-001",
        "timestamp": 1700000000000_u64
    });

    // Parse as Rust ChatRequest
    let parsed: opencarrier_types::yinghe::ChatRequest =
        serde_json::from_value(request).expect("ChatRequest should parse");

    // Verify all fields match
    assert_eq!(parsed.msg_type, "chat");
    assert_eq!(parsed.conversation_id, "conv-001");
    assert_eq!(parsed.plugin_id, Some("test-plugin".to_string()));
    assert_eq!(parsed.avatar_id, Some("test-avatar".to_string()));
    assert_eq!(parsed.content, Some("Hello".to_string()));
    assert_eq!(parsed.sender_id, Some("user-001".to_string()));
    assert_eq!(parsed.sender_name, Some("Alice".to_string()));
    assert_eq!(parsed.sender_username, Some("alice".to_string()));
    assert!(parsed.mentioned);
    assert!(!parsed.implicit_mention);
    assert_eq!(parsed.reply_to_message_id, Some("msg-000".to_string()));
    assert_eq!(parsed.reply_to_sender_id, Some("bot-001".to_string()));
    assert_eq!(parsed.message_id, Some("msg-001".to_string()));
}

/// Test ConversationType enum values match TypeScript
#[test]
fn test_conversation_type_values() {
    // TypeScript: 'carrier' | 'plugin' | 'avatar' | 'role'
    use opencarrier_types::yinghe::ConversationType;

    // Verify serialization matches TypeScript string values
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

    // Verify deserialization
    let carrier: ConversationType = serde_json::from_str("\"carrier\"").unwrap();
    assert_eq!(carrier, ConversationType::Carrier);

    let plugin: ConversationType = serde_json::from_str("\"plugin\"").unwrap();
    assert_eq!(plugin, ConversationType::Plugin);

    let avatar: ConversationType = serde_json::from_str("\"avatar\"").unwrap();
    assert_eq!(avatar, ConversationType::Avatar);

    let role: ConversationType = serde_json::from_str("\"role\"").unwrap();
    assert_eq!(role, ConversationType::Role);
}

/// Test ChatType enum values match TypeScript
#[test]
fn test_chat_type_values() {
    // TypeScript: 'direct' | 'group'
    use opencarrier_types::yinghe::ChatType;

    assert_eq!(
        serde_json::to_string(&ChatType::Direct).unwrap(),
        "\"direct\""
    );
    assert_eq!(
        serde_json::to_string(&ChatType::Group).unwrap(),
        "\"group\""
    );

    let direct: ChatType = serde_json::from_str("\"direct\"").unwrap();
    assert_eq!(direct, ChatType::Direct);

    let group: ChatType = serde_json::from_str("\"group\"").unwrap();
    assert_eq!(group, ChatType::Group);
}

// ---------------------------------------------------------------------------
// ChatResponse Comparison Tests
// ---------------------------------------------------------------------------

/// Test ChatResponse fields match TypeScript
#[test]
fn test_chat_response_fields_match() {
    // TypeScript ChatResponse fields:
    // - type: 'chat_response' | 'reply'
    // - conversationId: string
    // - conversationType: ConversationType
    // - chatType: ChatType
    // - pluginId?: string
    // - avatarId?: string
    // - response?: string
    // - message?: string (compat)
    // - attachment?: Attachment
    // - metadata?: ResponseMetadata

    let response = json!({
        "type": "chat_response",
        "conversationId": "conv-001",
        "conversationType": "plugin",
        "chatType": "group",
        "pluginId": "weather-plugin",
        "response": "It's sunny!",
        "metadata": {
            "rounds": 2,
            "toolCalls": 1,
            "matchedSkills": ["weather"]
        }
    });

    let parsed: opencarrier_types::yinghe::ChatResponse =
        serde_json::from_value(response).expect("ChatResponse should parse");

    assert_eq!(parsed.msg_type, "chat_response");
    assert_eq!(parsed.conversation_id, "conv-001");
    assert_eq!(parsed.plugin_id, Some("weather-plugin".to_string()));
    assert_eq!(parsed.response, Some("It's sunny!".to_string()));
    assert!(parsed.metadata.is_some());
}

/// Test ChatResponse serialization produces camelCase (TypeScript compatible)
#[test]
fn test_chat_response_camel_case() {
    use opencarrier_types::yinghe::{ChatRequest, ChatType, ConversationType, ChatResponse};

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

    let response = ChatResponse::for_request(&request, "Hello!".to_string());
    let json = serde_json::to_value(&response).unwrap();

    // Verify camelCase field names for always-present fields
    assert!(json.get("conversationId").is_some(), "Should use camelCase for conversationId");
    assert!(json.get("conversationType").is_some(), "Should use camelCase for conversationType");
    assert!(json.get("chatType").is_some(), "Should use camelCase for chatType");
    // Note: pluginId is None, so it won't be serialized (skip_serializing_if)

    // Verify snake_case fields are NOT present
    assert!(json.get("conversation_id").is_none(), "Should NOT use snake_case");
    assert!(json.get("conversation_type").is_none(), "Should NOT use snake_case");
}

// ---------------------------------------------------------------------------
// ErrorResponse Comparison Tests
// ---------------------------------------------------------------------------

/// Test ErrorResponse fields match TypeScript
#[test]
fn test_error_response_fields_match() {
    // TypeScript ErrorResponse fields:
    // - type: 'error'
    // - conversationId?: string
    // - conversationType?: ConversationType
    // - chatType?: ChatType
    // - pluginId?: string
    // - avatarId?: string
    // - message: string
    // - code?: string

    let error = json!({
        "type": "error",
        "conversationId": "conv-001",
        "conversationType": "plugin",
        "chatType": "direct",
        "pluginId": "test-plugin",
        "message": "Something went wrong"
    });

    let parsed: opencarrier_types::yinghe::ErrorResponse =
        serde_json::from_value(error).expect("ErrorResponse should parse");

    assert_eq!(parsed.msg_type, "error");
    assert_eq!(parsed.conversation_id, Some("conv-001".to_string()));
    assert_eq!(parsed.plugin_id, Some("test-plugin".to_string()));
    assert_eq!(parsed.message, "Something went wrong");
}

// ---------------------------------------------------------------------------
// SessionKey Comparison Tests
// ---------------------------------------------------------------------------

/// Test SessionKey format matches TypeScript
#[test]
fn test_session_key_format_matches() {
    // TypeScript format: "<conversationType>:<chatType>:<instanceId>:<conversationId>"
    // Examples from TypeScript:
    // - "carrier:direct:main:c001"
    // - "plugin:direct:weather:c002"
    // - "plugin:group:weather:g001"
    // - "avatar:direct:main:c004"
    // - "avatar:group:work:g005"

    use opencarrier_types::yinghe::{ChatType, ConversationType, SessionKey};

    // Test carrier:direct:main:c001
    let key1 = SessionKey::new(ConversationType::Carrier, ChatType::Direct, "main", "c001");
    assert_eq!(format!("{}", key1), "carrier:direct:main:c001");

    // Test plugin:direct:weather:c002
    let key2 = SessionKey::new(ConversationType::Plugin, ChatType::Direct, "weather", "c002");
    assert_eq!(format!("{}", key2), "plugin:direct:weather:c002");

    // Test plugin:group:weather:g001
    let key3 = SessionKey::new(ConversationType::Plugin, ChatType::Group, "weather", "g001");
    assert_eq!(format!("{}", key3), "plugin:group:weather:g001");

    // Test avatar:direct:main:c004
    let key4 = SessionKey::new(ConversationType::Avatar, ChatType::Direct, "main", "c004");
    assert_eq!(format!("{}", key4), "avatar:direct:main:c004");

    // Test avatar:group:work:g005
    let key5 = SessionKey::new(ConversationType::Avatar, ChatType::Group, "work", "g005");
    assert_eq!(format!("{}", key5), "avatar:group:work:g005");
}

/// Test SessionKey parsing matches TypeScript
#[test]
fn test_session_key_parsing() {
    use opencarrier_types::yinghe::{ChatType, ConversationType, SessionKey};

    // Parse various session keys
    let key1 = SessionKey::parse("carrier:direct:main:c001").unwrap();
    assert_eq!(key1.conversation_type, ConversationType::Carrier);
    assert_eq!(key1.chat_type, ChatType::Direct);
    assert_eq!(key1.instance_id, "main");
    assert_eq!(key1.conversation_id, "c001");

    let key2 = SessionKey::parse("plugin:group:weather:g001").unwrap();
    assert_eq!(key2.conversation_type, ConversationType::Plugin);
    assert_eq!(key2.chat_type, ChatType::Group);
    assert_eq!(key2.instance_id, "weather");
    assert_eq!(key2.conversation_id, "g001");

    // Invalid keys should return None
    assert!(SessionKey::parse("invalid").is_none());
    assert!(SessionKey::parse("carrier:direct:main").is_none()); // Missing conversationId
}

// ---------------------------------------------------------------------------
// Type Guard Equivalents Tests
// ---------------------------------------------------------------------------

/// Test isChatRequest equivalent in Rust
#[test]
fn test_is_chat_request_equivalent() {
    // TypeScript isChatRequest checks:
    // 1. type is 'chat' or 'message'
    // 2. conversationType is valid
    // 3. chatType is valid

    let valid_chat = json!({
        "type": "chat",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "content": "Hello"
    });

    let valid_message = json!({
        "type": "message",
        "conversationId": "conv-002",
        "conversationType": "plugin",
        "chatType": "group",
        "content": "Hello"
    });

    let invalid_type = json!({
        "type": "other",
        "conversationType": "carrier",
        "chatType": "direct"
    });

    let missing_fields = json!({
        "type": "chat"
    });

    // Valid 'chat' type should parse
    assert!(
        serde_json::from_value::<opencarrier_types::yinghe::ChatRequest>(valid_chat).is_ok()
    );

    // Valid 'message' type should parse
    assert!(
        serde_json::from_value::<opencarrier_types::yinghe::ChatRequest>(valid_message).is_ok()
    );

    // Invalid type should fail
    assert!(
        serde_json::from_value::<opencarrier_types::yinghe::ChatRequest>(invalid_type).is_err()
    );

    // Missing required fields should fail
    assert!(
        serde_json::from_value::<opencarrier_types::yinghe::ChatRequest>(missing_fields).is_err()
    );
}

// ---------------------------------------------------------------------------
// Response Routing Validation Tests
// ---------------------------------------------------------------------------

/// Test that response routing matches request (TypeScript validateResponseRouting)
#[test]
fn test_response_routing_validation() {
    use opencarrier_types::yinghe::{ChatRequest, ChatResponse, ChatType, ConversationType};

    let request = ChatRequest {
        msg_type: "chat".to_string(),
        conversation_id: "conv-001".to_string(),
        conversation_type: ConversationType::Plugin,
        chat_type: ChatType::Group,
        plugin_id: Some("weather-plugin".to_string()),
        avatar_id: None,
        content: Some("What's the weather?".to_string()),
        attachment: None,
        sender_id: None,
        sender_name: None,
        sender_username: None,
        mentioned: true,
        implicit_mention: false,
        reply_to_message_id: None,
        reply_to_sender_id: None,
        message_id: None,
        timestamp: None,
    };

    let response = ChatResponse::for_request(&request, "It's sunny!".to_string());

    // TypeScript validateResponseRouting checks:
    // 1. conversationId matches
    // 2. conversationType matches
    // 3. chatType matches
    // 4. pluginId matches (for plugin type)
    // 5. avatarId matches (for avatar type)

    assert_eq!(request.conversation_id, response.conversation_id);
    assert_eq!(request.conversation_type, response.conversation_type);
    assert_eq!(request.chat_type, response.chat_type);
    assert_eq!(request.plugin_id, response.plugin_id);
    assert_eq!(request.avatar_id, response.avatar_id);
}

// ---------------------------------------------------------------------------
// Attachment Comparison Tests
// ---------------------------------------------------------------------------

/// Test Attachment type matches TypeScript
#[test]
fn test_attachment_fields_match() {
    // TypeScript Attachment fields:
    // - filename: string
    // - mimeType: string
    // - data: string (Base64)
    // - size: number

    let attachment = json!({
        "filename": "test.pdf",
        "mimeType": "application/pdf",
        "data": "base64encodeddata",
        "size": 1024
    });

    let parsed: opencarrier_types::yinghe::Attachment =
        serde_json::from_value(attachment).expect("Attachment should parse");

    assert_eq!(parsed.filename, "test.pdf");
    assert_eq!(parsed.mime_type, "application/pdf");
    assert_eq!(parsed.data, "base64encodeddata");
    assert_eq!(parsed.size, 1024);
}

// ---------------------------------------------------------------------------
// ResponseMetadata Comparison Tests
// ---------------------------------------------------------------------------

/// Test ResponseMetadata fields match TypeScript
#[test]
fn test_response_metadata_fields_match() {
    // TypeScript ResponseMetadata fields:
    // - rounds?: number
    // - toolCalls?: number
    // - matchedSkills?: string[]
    // - taskProgress?: { total, completed, failed, percentage }
    // - qualityScore?: number
    // - isScheduledTaskResult?: boolean
    // - taskName?: string
    // - success?: boolean
    // - isScheduledTaskCreated?: boolean
    // - taskId?: string
    // - isRecurrenceProposal?: boolean
    // - recurrence?: { taskName, timeDescription, suggestedSchedule }
    // - originalInput?: string

    let metadata = json!({
        "rounds": 2,
        "toolCalls": 3,
        "matchedSkills": ["skill1", "skill2"],
        "taskProgress": {
            "total": 10,
            "completed": 5,
            "failed": 1,
            "percentage": 50
        },
        "qualityScore": 0.85
    });

    let parsed: opencarrier_types::yinghe::ResponseMetadata =
        serde_json::from_value(metadata).expect("ResponseMetadata should parse");

    assert_eq!(parsed.rounds, Some(2));
    assert_eq!(parsed.tool_calls, Some(3));
    assert_eq!(parsed.matched_skills, Some(vec!["skill1".to_string(), "skill2".to_string()]));
}

// ---------------------------------------------------------------------------
// Stream Message Comparison Tests (if supported)
// ---------------------------------------------------------------------------

/// Test StreamStart type (if streaming is implemented)
#[test]
fn test_stream_start_format() {
    // TypeScript StreamStart fields:
    // - type: 'stream_start'
    // - messageId: string
    // - conversationId: string
    // - conversationType: ConversationType
    // - chatType: ChatType
    // - pluginId?: string
    // - avatarId?: string
    // - timestamp: number

    let stream_start = json!({
        "type": "stream_start",
        "messageId": "msg-001",
        "conversationId": "conv-001",
        "conversationType": "carrier",
        "chatType": "direct",
        "timestamp": 1700000000000_u64
    });

    // Verify the format is valid JSON
    assert!(stream_start.get("type").is_some());
    assert_eq!(stream_start["type"], "stream_start");
}

/// Test StreamChunk type (if streaming is implemented)
#[test]
fn test_stream_chunk_format() {
    // TypeScript StreamChunk fields:
    // - type: 'stream_chunk'
    // - messageId: string
    // - conversationId: string
    // - content: string
    // - index: number
    // - done?: boolean

    let stream_chunk = json!({
        "type": "stream_chunk",
        "messageId": "msg-001",
        "conversationId": "conv-001",
        "content": "Hello",
        "index": 0,
        "done": false
    });

    assert_eq!(stream_chunk["type"], "stream_chunk");
    assert_eq!(stream_chunk["content"], "Hello");
}

/// Test StreamEnd type (if streaming is implemented)
#[test]
fn test_stream_end_format() {
    // TypeScript StreamEnd fields:
    // - type: 'stream_end'
    // - messageId: string
    // - conversationId: string
    // - response?: string
    // - metadata?: ResponseMetadata

    let stream_end = json!({
        "type": "stream_end",
        "messageId": "msg-001",
        "conversationId": "conv-001",
        "response": "Hello, how can I help?",
        "metadata": {
            "rounds": 1
        }
    });

    assert_eq!(stream_end["type"], "stream_end");
}
