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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiFormat {
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
    fn test_api_format_serde() {
        assert_eq!(serde_json::to_string(&ApiFormat::OpenAI).unwrap(), "\"openai\"");
        assert_eq!(serde_json::to_string(&ApiFormat::Anthropic).unwrap(), "\"anthropic\"");
        assert_eq!(serde_json::to_string(&ApiFormat::Gemini).unwrap(), "\"gemini\"");

        let f: ApiFormat = serde_json::from_str("\"anthropic\"").unwrap();
        assert_eq!(f, ApiFormat::Anthropic);
    }
}
