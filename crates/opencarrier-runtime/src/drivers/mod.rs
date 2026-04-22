//! LLM driver implementations.
//!
//! Contains drivers for Anthropic Claude, Google Gemini, OpenAI-compatible APIs, and more.
//! Supports: Anthropic, Gemini, OpenAI, Groq, OpenRouter, DeepSeek, Together,
//! Mistral, Fireworks, Ollama, vLLM, Chutes.ai, Cloud Proxy, and any OpenAI-compatible endpoint.

pub mod anthropic;
pub mod claude_code;
pub mod fallback;
pub mod gemini;
pub mod openai;
pub mod qwen_code;

use crate::llm_driver::{DriverConfig, LlmDriver, LlmError};
use opencarrier_types::brain::ApiFormat;
use std::sync::Arc;

/// Create an LLM driver based on the format field in configuration.
///
/// The `format` field (from brain.json) determines the driver type:
/// - `anthropic` → Anthropic Claude Messages API driver
/// - `gemini`    → Google Gemini generateContent API driver
/// - `openai`    → OpenAI-compatible driver (covers Groq, OpenRouter, DeepSeek, Ollama, etc.)
///
/// `base_url` is used as-is — it must be the complete API endpoint URL.
/// No path suffix is appended by any driver.
pub fn create_driver(config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
    let provider = config.provider.as_str();

    // CLI subprocess drivers (not HTTP API) — dispatched by provider name
    if provider == "claude-code" {
        return Ok(Arc::new(claude_code::ClaudeCodeDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        )));
    }
    if provider == "qwen-code" {
        return Ok(Arc::new(qwen_code::QwenCodeDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        )));
    }

    // HTTP API drivers — dispatched by format
    let format = config.format.unwrap_or_default();

    match format {
        ApiFormat::Anthropic => {
            let api_key = config.api_key.clone().ok_or_else(|| {
                LlmError::MissingApiKey("API key required for Anthropic format".to_string())
            })?;
            let base_url = config.base_url.clone().ok_or_else(|| LlmError::Api {
                status: 0,
                message: "base_url required for Anthropic format".to_string(),
            })?;
            Ok(Arc::new(anthropic::AnthropicDriver::new(api_key, base_url)))
        }
        ApiFormat::Gemini => {
            let api_key = config.api_key.clone().ok_or_else(|| {
                LlmError::MissingApiKey("API key required for Gemini format".to_string())
            })?;
            let base_url = config.base_url.clone().ok_or_else(|| LlmError::Api {
                status: 0,
                message: "base_url required for Gemini format".to_string(),
            })?;
            Ok(Arc::new(gemini::GeminiDriver::new(api_key, base_url)))
        }
        ApiFormat::OpenAI => {
            let api_key = config.api_key.clone().unwrap_or_default();
            let base_url = config.base_url.clone().ok_or_else(|| LlmError::Api {
                status: 0,
                message: "base_url required for OpenAI format".to_string(),
            })?;
            Ok(Arc::new(openai::OpenAIDriver::with_auth_header(
                api_key,
                base_url,
                config.auth_header,
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencarrier_types::brain::AuthHeaderType;

    #[test]
    fn test_anthropic_format_with_key_and_url() {
        let config = DriverConfig {
            provider: "anthropic".to_string(),
            api_key: Some("test-key".to_string()),
            base_url: Some("https://api.anthropic.com/v1/messages".to_string()),
            format: Some(ApiFormat::Anthropic),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "Anthropic format with key + URL should succeed");
    }

    #[test]
    fn test_anthropic_format_no_key_errors() {
        let config = DriverConfig {
            provider: "anthropic".to_string(),
            api_key: None,
            base_url: Some("https://api.anthropic.com/v1/messages".to_string()),
            format: Some(ApiFormat::Anthropic),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err(), "Anthropic format without key should error");
    }

    #[test]
    fn test_anthropic_format_no_url_errors() {
        let config = DriverConfig {
            provider: "anthropic".to_string(),
            api_key: Some("test-key".to_string()),
            base_url: None,
            format: Some(ApiFormat::Anthropic),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err(), "Anthropic format without URL should error");
    }

    #[test]
    fn test_openai_format_with_key_and_url() {
        let config = DriverConfig {
            provider: "groq".to_string(),
            api_key: Some("test-key".to_string()),
            base_url: Some("https://api.groq.com/openai/v1/chat/completions".to_string()),
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "OpenAI format with key + URL should succeed");
    }

    #[test]
    fn test_openai_format_no_key_succeeds() {
        // OpenAI format does not require API key (e.g. Ollama)
        let config = DriverConfig {
            provider: "ollama".to_string(),
            api_key: None,
            base_url: Some("http://localhost:11434/v1/chat/completions".to_string()),
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "OpenAI format without key should succeed (local providers)");
    }

    #[test]
    fn test_openai_format_no_url_errors() {
        let config = DriverConfig {
            provider: "openai".to_string(),
            api_key: Some("test-key".to_string()),
            base_url: None,
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err(), "OpenAI format without URL should error");
    }

    #[test]
    fn test_gemini_format_with_key_and_url() {
        let config = DriverConfig {
            provider: "gemini".to_string(),
            api_key: Some("test-key".to_string()),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta/models".to_string()),
            format: Some(ApiFormat::Gemini),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "Gemini format with key + URL should succeed");
    }

    #[test]
    fn test_gemini_format_no_key_errors() {
        let config = DriverConfig {
            provider: "gemini".to_string(),
            api_key: None,
            base_url: Some("https://generativelanguage.googleapis.com".to_string()),
            format: Some(ApiFormat::Gemini),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err(), "Gemini format without key should error");
    }

    #[test]
    fn test_azure_driver_with_key_and_url() {
        let config = DriverConfig {
            provider: "azure".to_string(),
            api_key: Some("test-azure-key".to_string()),
            base_url: Some(
                "https://myresource.openai.azure.com/openai/deployments".to_string(),
            ),
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "Azure driver with key + URL should succeed");
    }

    #[test]
    fn test_azure_driver_no_url_errors() {
        let config = DriverConfig {
            provider: "azure".to_string(),
            api_key: Some("test-azure-key".to_string()),
            base_url: None,
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err(), "Azure driver without URL should error");
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("base_url"),
            "Error should mention base_url: {}",
            err
        );
    }

    #[test]
    fn test_azure_openai_alias_driver_creation() {
        let config = DriverConfig {
            provider: "azure-openai".to_string(),
            api_key: Some("test-azure-key".to_string()),
            base_url: Some(
                "https://myresource.openai.azure.com/openai/deployments".to_string(),
            ),
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "azure-openai alias should create driver successfully"
        );
    }

    #[test]
    fn test_kimi_coding_anthropic_format() {
        // kimi_coding with Anthropic format should use AnthropicDriver
        let config = DriverConfig {
            provider: "kimi".to_string(),
            api_key: Some("test-kimi-key".to_string()),
            base_url: Some("https://api.kimi.com/coding/v1/messages".to_string()),
            format: Some(ApiFormat::Anthropic),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "kimi_coding with Anthropic format should succeed"
        );
    }

    #[test]
    fn test_claude_code_cli_driver() {
        let config = DriverConfig {
            provider: "claude-code".to_string(),
            api_key: None,
            base_url: Some("/usr/local/bin/claude".to_string()),
            format: None,
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "claude-code provider should create CLI driver"
        );
    }

    #[test]
    fn test_unknown_provider_openai_format() {
        // Any provider with OpenAI format and base_url should work
        let config = DriverConfig {
            provider: "my-custom-llm".to_string(),
            api_key: Some("test".to_string()),
            base_url: Some("http://localhost:9999/v1/chat/completions".to_string()),
            format: Some(ApiFormat::OpenAI),
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_default_format_is_openai() {
        // When format is None, defaults to OpenAI
        let config = DriverConfig {
            provider: "custom".to_string(),
            api_key: Some("test".to_string()),
            base_url: Some("http://localhost:1234/v1/chat/completions".to_string()),
            format: None,
            auth_header: AuthHeaderType::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok(), "Default format (OpenAI) should work");
    }
}
