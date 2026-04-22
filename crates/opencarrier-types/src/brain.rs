//! Brain configuration types — the carrier's LLM brain.
//!
//! Three-layer architecture:
//! - **Provider**: identity + credentials (name + API key)
//! - **Endpoint**: complete callable unit (provider + model + base_url + format)
//! - **Modality**: task type → endpoint with fallback chain

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level brain configuration, deserialized from `brain.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    /// Providers: name → credentials.
    pub providers: HashMap<String, ProviderConfig>,
    /// Endpoints: name → complete callable unit.
    pub endpoints: HashMap<String, EndpointConfig>,
    /// Modalities: task type → endpoint routing.
    pub modalities: HashMap<String, ModalityConfig>,
    /// Default modality when agent doesn't specify one.
    #[serde(default = "default_modality")]
    pub default_modality: String,
}

fn default_modality() -> String {
    "chat".to_string()
}

/// Provider = identity + credentials.
///
/// Only knows name and how to authenticate. No URLs, no formats, no models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Environment variable name holding the API key.
    /// If empty/missing, this provider doesn't require authentication (e.g., Ollama).
    #[serde(default)]
    pub api_key_env: String,
    /// Authentication type: "apikey" (default), "jwt", etc.
    /// Determines how credentials are used to authenticate API calls.
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    /// Additional parameters (env var names) for multi-credential providers.
    /// Maps a logical name → environment variable name to read at runtime.
    /// Example: Kling needs access_key + secret_key:
    ///   { "access_key_env": "KLING_ACCESS_KEY", "secret_key_env": "KLING_SECRET_KEY" }
    #[serde(default)]
    pub params: HashMap<String, String>,
}

fn default_auth_type() -> String {
    "apikey".to_string()
}

/// Endpoint = format + base_url + model (complete callable unit).
///
/// Contains everything needed to make an LLM API call:
/// - Which provider to get credentials from
/// - Which model to request
/// - Where to send the request (base_url)
/// - How to format the request/response (format/protocol)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// Provider name — used to look up API key.
    pub provider: String,
    /// Model identifier (e.g., "glm-5.1", "deepseek-chat").
    pub model: String,
    /// Complete API base URL.
    pub base_url: String,
    /// Protocol format: determines which driver to use.
    /// "openai" → OpenAIDriver, "anthropic" → AnthropicDriver, "gemini" → GeminiDriver
    #[serde(default = "default_format")]
    pub format: ApiFormat,
}

fn default_format() -> ApiFormat {
    ApiFormat::OpenAI
}

/// API protocol format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiFormat {
    #[default]
    OpenAI,
    Anthropic,
    Gemini,
}

impl std::fmt::Display for ApiFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiFormat::OpenAI => write!(f, "openai"),
            ApiFormat::Anthropic => write!(f, "anthropic"),
            ApiFormat::Gemini => write!(f, "gemini"),
        }
    }
}

/// Modality = task type → endpoint routing.
///
/// Maps a capability (chat, vision, code, etc.) to a primary endpoint
/// with optional fallback chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalityConfig {
    /// Primary endpoint name.
    pub primary: String,
    /// Fallback endpoint names, tried in order on failure.
    #[serde(default)]
    pub fallbacks: Vec<String>,
    /// Human-readable description of this modality.
    #[serde(default)]
    pub description: String,
}

// ---------------------------------------------------------------------------
// Brain query types — returned by the Brain trait methods
// ---------------------------------------------------------------------------

/// A resolved endpoint returned by `Brain::endpoints_for()`.
///
/// Contains everything the runtime needs to call this endpoint,
/// without the driver itself (driver is fetched separately via
/// `Brain::driver_for_endpoint()`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedEndpoint {
    /// Endpoint name (the key from brain.json endpoints).
    pub id: String,
    /// Model name to set in CompletionRequest.model.
    pub model: String,
    /// Provider name (for logging / health tracking).
    pub provider: String,
}

/// Information about a modality, returned by `Brain::list_modalities()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalityInfo {
    /// Modality name (e.g., "chat", "fast", "vision").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Primary endpoint name.
    pub primary_endpoint: String,
    /// Number of fallback endpoints.
    pub fallback_count: usize,
}

/// Feedback from the runtime to Brain after an endpoint call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointReport {
    /// Which endpoint was attempted.
    pub endpoint_id: String,
    /// Whether the call succeeded.
    pub success: bool,
    /// Call latency in milliseconds.
    pub latency_ms: u64,
    /// Error message if the call failed.
    pub error: Option<String>,
}

/// Health status of a single endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointHealth {
    /// Endpoint name.
    pub endpoint: String,
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Whether the driver was successfully created at boot.
    pub driver_ready: bool,
    /// Total successful calls (from report()).
    pub success_count: u64,
    /// Total failed calls (from report()).
    pub failure_count: u64,
    /// Average latency in ms (0 if no data).
    pub avg_latency_ms: u64,
    /// Consecutive failures (reset to 0 on success).
    pub consecutive_failures: u32,
    /// Whether the circuit-breaker has opened (endpoint taken out of rotation).
    pub circuit_open: bool,
}

/// Overall Brain status snapshot, returned by `Brain::status()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainStatus {
    /// All modalities.
    pub modalities: Vec<ModalityInfo>,
    /// Health of all endpoints.
    pub endpoints: Vec<EndpointHealth>,
    /// Number of drivers that initialized successfully.
    pub drivers_ready: usize,
}

/// Resolved credentials for a provider, ready for injection into skill subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredentials {
    /// Provider name.
    pub provider_name: String,
    /// Environment variable name → resolved value pairs.
    /// e.g., {"KLING_ACCESS_KEY": "xxx", "KLING_SECRET_KEY": "yyy"}
    pub env_vars: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brain_config_parse() {
        let json = r#"{
            "providers": {
                "zhipu": { "api_key_env": "ANTHROPIC_API_KEY" },
                "deepseek": { "api_key_env": "DEEPSEEK_API_KEY" },
                "ollama": {}
            },
            "endpoints": {
                "zhipu_anthropic": {
                    "provider": "zhipu",
                    "model": "glm-5.1",
                    "base_url": "https://open.bigmodel.cn/api/anthropic",
                    "format": "anthropic"
                },
                "deepseek_chat": {
                    "provider": "deepseek",
                    "model": "deepseek-chat",
                    "base_url": "https://api.deepseek.com/v1",
                    "format": "openai"
                },
                "ollama_local": {
                    "provider": "ollama",
                    "model": "llama3.2:latest",
                    "base_url": "http://localhost:11434/v1"
                }
            },
            "modalities": {
                "chat": {
                    "primary": "zhipu_anthropic",
                    "fallbacks": ["deepseek_chat"]
                },
                "fast": {
                    "primary": "ollama_local"
                }
            }
        }"#;

        let config: BrainConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.providers.len(), 3);
        assert_eq!(config.providers["zhipu"].api_key_env, "ANTHROPIC_API_KEY");
        assert!(config.providers["ollama"].api_key_env.is_empty());

        assert_eq!(config.endpoints.len(), 3);
        assert_eq!(config.endpoints["zhipu_anthropic"].model, "glm-5.1");
        assert_eq!(config.endpoints["zhipu_anthropic"].format, ApiFormat::Anthropic);
        assert_eq!(config.endpoints["ollama_local"].format, ApiFormat::OpenAI); // default

        assert_eq!(config.modalities["chat"].primary, "zhipu_anthropic");
        assert_eq!(config.modalities["chat"].fallbacks, vec!["deepseek_chat"]);
        assert!(config.modalities["fast"].fallbacks.is_empty());

        assert_eq!(config.default_modality, "chat"); // default
    }

    #[test]
    fn test_provider_auth_type_default() {
        // Missing auth_type defaults to "apikey"
        let json = r#"{"api_key_env": "FOO"}"#;
        let pc: ProviderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(pc.auth_type, "apikey");

        // Explicit auth_type
        let json = r#"{"auth_type": "jwt", "params": {"access_key_env": "AK", "secret_key_env": "SK"}}"#;
        let pc: ProviderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(pc.auth_type, "jwt");
        assert_eq!(pc.params["access_key_env"], "AK");
    }

    #[test]
    fn test_api_format_serde() {
        assert_eq!(serde_json::to_string(&ApiFormat::OpenAI).unwrap(), "\"openai\"");
        assert_eq!(serde_json::to_string(&ApiFormat::Anthropic).unwrap(), "\"anthropic\"");
        assert_eq!(serde_json::to_string(&ApiFormat::Gemini).unwrap(), "\"gemini\"");

        let f: ApiFormat = serde_json::from_str("\"anthropic\"").unwrap();
        assert_eq!(f, ApiFormat::Anthropic);
    }
}
