//! yingheclient-compatible session management.
//!
//! Provides session persistence for yingheclient protocol messages,
//! mapping SessionKey to OpenCarrier's SessionStore.

use crate::session::{Session, SessionStore};
use opencarrier_types::agent::AgentId;
use opencarrier_types::message::Message;
use opencarrier_types::yinghe::{ChatRequest, ChatResponse, SessionKey};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Session key to SessionId mapping store.
const CREATE_SESSION_MAP_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS yinghe_session_map (
    session_key TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// yingheclient session manager that wraps SessionStore.
#[derive(Clone)]
pub struct YingheSessionManager {
    /// Underlying session store.
    session_store: SessionStore,
    /// Connection for session key mapping.
    conn: Arc<Mutex<Connection>>,
    /// In-memory cache for session key -> session id mapping.
    cache: Arc<Mutex<HashMap<String, opencarrier_types::agent::SessionId>>>,
    /// Default agent ID for new sessions.
    default_agent_id: AgentId,
}

impl YingheSessionManager {
    /// Create a new yingheclient session manager.
    pub fn new(conn: Arc<Mutex<Connection>>, default_agent_id: AgentId) -> Result<Self, String> {
        // Create the session map table
        {
            let locked = conn.lock().map_err(|e| e.to_string())?;
            locked
                .execute(CREATE_SESSION_MAP_TABLE, [])
                .map_err(|e| e.to_string())?;
        }

        Ok(Self {
            session_store: SessionStore::new(conn.clone()),
            conn,
            cache: Arc::new(Mutex::new(HashMap::new())),
            default_agent_id,
        })
    }

    /// Get or create a session for the given session key.
    pub fn get_or_create_session(&self, key: &SessionKey) -> Result<Session, String> {
        let key_str = key.to_string();

        // Check cache first
        {
            let cache = self.cache.lock().map_err(|e| e.to_string())?;
            if let Some(&session_id) = cache.get(&key_str) {
                if let Some(session) = self
                    .session_store
                    .get_session(session_id)
                    .map_err(|e| e.to_string())?
                {
                    return Ok(session);
                }
            }
        }

        // Check database
        let session_id = self.lookup_session_id(&key_str)?;

        if let Some(sid) = session_id {
            // Update cache
            {
                let mut cache = self.cache.lock().map_err(|e| e.to_string())?;
                cache.insert(key_str.clone(), sid);
            }

            if let Some(session) = self
                .session_store
                .get_session(sid)
                .map_err(|e| e.to_string())?
            {
                return Ok(session);
            }
        }

        // Create new session
        self.create_new_session(key)
    }

    /// Create a new session for the given key.
    fn create_new_session(&self, key: &SessionKey) -> Result<Session, String> {
        let key_str = key.to_string();
        let session = self
            .session_store
            .create_session_with_label(self.default_agent_id, Some(&key_str))
            .map_err(|e| e.to_string())?;

        // Store mapping
        self.store_session_mapping(&key_str, session.id, self.default_agent_id)?;

        // Update cache
        {
            let mut cache = self.cache.lock().map_err(|e| e.to_string())?;
            cache.insert(key_str, session.id);
        }

        Ok(session)
    }

    /// Look up session ID by session key.
    fn lookup_session_id(
        &self,
        key: &str,
    ) -> Result<Option<opencarrier_types::agent::SessionId>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT session_id FROM yinghe_session_map WHERE session_key = ?1")
            .map_err(|e| e.to_string())?;

        let result = stmt.query_row(rusqlite::params![key], |row| {
            let id_str: String = row.get(0)?;
            Ok(id_str)
        });

        match result {
            Ok(id_str) => {
                let uuid = uuid::Uuid::parse_str(&id_str).map_err(|e| e.to_string())?;
                Ok(Some(opencarrier_types::agent::SessionId(uuid)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Store session key to session ID mapping.
    fn store_session_mapping(
        &self,
        key: &str,
        session_id: opencarrier_types::agent::SessionId,
        agent_id: AgentId,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR REPLACE INTO yinghe_session_map (session_key, session_id, agent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![key, session_id.to_string(), agent_id.to_string(), now],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Add a message to the session.
    pub fn add_message(&self, key: &SessionKey, message: Message) -> Result<(), String> {
        let mut session = self.get_or_create_session(key)?;
        session.messages.push(message);
        self.session_store
            .save_session(&session)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Add user message from ChatRequest.
    pub fn add_user_message(&self, request: &ChatRequest) -> Result<(), String> {
        let key = request.session_key();
        let content = request.text_content().to_string();
        self.add_message(&key, Message::user(content))
    }

    /// Add assistant message from ChatResponse.
    pub fn add_assistant_message(&self, request: &ChatRequest, response: &ChatResponse) -> Result<(), String> {
        let key = request.session_key();
        let content = response
            .response
            .as_ref()
            .or(response.message.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("");

        if !content.is_empty() {
            self.add_message(&key, Message::assistant(content.to_string()))?;
        }
        Ok(())
    }

    /// Get conversation history for a session.
    pub fn get_history(&self, key: &SessionKey) -> Result<Vec<Message>, String> {
        let session = self.get_or_create_session(key)?;
        Ok(session.messages)
    }

    /// Get conversation history with limit.
    pub fn get_history_with_limit(&self, key: &SessionKey, limit: usize) -> Result<Vec<Message>, String> {
        let messages = self.get_history(key)?;
        if messages.len() <= limit {
            return Ok(messages);
        }
        // Return the most recent messages
        Ok(messages[messages.len() - limit..].to_vec())
    }

    /// Clear session history.
    pub fn clear_session(&self, key: &SessionKey) -> Result<(), String> {
        let key_str = key.to_string();

        // Get session ID
        if let Some(session_id) = self.lookup_session_id(&key_str)? {
            // Delete session
            self.session_store
                .delete_session(session_id)
                .map_err(|e| e.to_string())?;

            // Delete mapping
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "DELETE FROM yinghe_session_map WHERE session_key = ?1",
                rusqlite::params![key_str],
            )
            .map_err(|e| e.to_string())?;

            // Update cache
            {
                let mut cache = self.cache.lock().map_err(|e| e.to_string())?;
                cache.remove(&key_str);
            }
        }

        Ok(())
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT session_key, session_id, agent_id, created_at, updated_at
                 FROM yinghe_session_map ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let session_key: String = row.get(0)?;
                let session_id: String = row.get(1)?;
                let agent_id: String = row.get(2)?;
                let created_at: String = row.get(3)?;
                let updated_at: String = row.get(4)?;
                Ok(SessionInfo {
                    session_key,
                    session_id,
                    agent_id,
                    created_at,
                    updated_at,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| e.to_string())?);
        }
        Ok(sessions)
    }

    /// Get session store reference.
    pub fn session_store(&self) -> &SessionStore {
        &self.session_store
    }
}

/// Session info for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_key: String,
    pub session_id: String,
    pub agent_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencarrier_types::yinghe::{ChatType, ConversationType};
    use rusqlite::Connection;

    fn setup() -> YingheSessionManager {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::run_migrations(&conn).unwrap();
        YingheSessionManager::new(
            Arc::new(Mutex::new(conn)),
            AgentId::new(),
        ).unwrap()
    }

    #[test]
    fn test_create_session() {
        let manager = setup();
        let key = SessionKey::new(
            ConversationType::Carrier,
            ChatType::Direct,
            "main",
            "conv-001",
        );

        let session = manager.get_or_create_session(&key).unwrap();
        assert!(session.messages.is_empty());
        assert!(session.label.as_ref().unwrap().contains("carrier:direct:main:conv-001"));
    }

    #[test]
    fn test_add_message() {
        let manager = setup();
        let key = SessionKey::new(
            ConversationType::Carrier,
            ChatType::Direct,
            "main",
            "conv-002",
        );

        manager.add_message(&key, Message::user("Hello")).unwrap();
        manager.add_message(&key, Message::assistant("Hi there!")).unwrap();

        let history = manager.get_history(&key).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content.text_content(), "Hello");
        assert_eq!(history[1].content.text_content(), "Hi there!");
    }

    #[test]
    fn test_session_persistence() {
        let manager = setup();
        let key = SessionKey::new(
            ConversationType::Plugin,
            ChatType::Group,
            "weather",
            "group-001",
        );

        manager.add_message(&key, Message::user("What's the weather?")).unwrap();

        // Get session again - should have the message
        let history = manager.get_history(&key).unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_history_with_limit() {
        let manager = setup();
        let key = SessionKey::new(
            ConversationType::Carrier,
            ChatType::Direct,
            "main",
            "conv-003",
        );

        // Add 10 messages
        for i in 0..10 {
            manager.add_message(&key, Message::user(format!("Message {}", i))).unwrap();
        }

        // Get last 5
        let history = manager.get_history_with_limit(&key, 5).unwrap();
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].content.text_content(), "Message 5");
        assert_eq!(history[4].content.text_content(), "Message 9");
    }

    #[test]
    fn test_clear_session() {
        let manager = setup();
        let key = SessionKey::new(
            ConversationType::Carrier,
            ChatType::Direct,
            "main",
            "conv-004",
        );

        manager.add_message(&key, Message::user("Hello")).unwrap();
        assert!(!manager.get_history(&key).unwrap().is_empty());

        manager.clear_session(&key).unwrap();
        assert!(manager.get_history(&key).unwrap().is_empty());
    }

    #[test]
    fn test_add_user_message_from_request() {
        let manager = setup();
        let request = ChatRequest {
            msg_type: "chat".to_string(),
            conversation_id: "conv-005".to_string(),
            conversation_type: ConversationType::Carrier,
            chat_type: ChatType::Direct,
            plugin_id: None,
            avatar_id: None,
            content: Some("Hello from request".to_string()),
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

        manager.add_user_message(&request).unwrap();

        let key = request.session_key();
        let history = manager.get_history(&key).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.text_content(), "Hello from request");
    }
}
