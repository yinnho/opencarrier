//! Model catalog types — shared data structures for the model registry.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Canonical provider base URLs — single source of truth.
// Referenced by opencarrier-runtime drivers, model catalog, and embedding modules.
// ---------------------------------------------------------------------------

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";
pub const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";
pub const TOGETHER_BASE_URL: &str = "https://api.together.xyz/v1";
pub const FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
pub const VLLM_BASE_URL: &str = "http://localhost:8000/v1";
pub const LMSTUDIO_BASE_URL: &str = "http://localhost:1234/v1";
pub const LEMONADE_BASE_URL: &str = "http://localhost:8888/api/v1";
pub const PERPLEXITY_BASE_URL: &str = "https://api.perplexity.ai";
pub const COHERE_BASE_URL: &str = "https://api.cohere.com/v2";
pub const AI21_BASE_URL: &str = "https://api.ai21.com/studio/v1";
pub const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";
pub const SAMBANOVA_BASE_URL: &str = "https://api.sambanova.ai/v1";
pub const HUGGINGFACE_BASE_URL: &str = "https://api-inference.huggingface.co/v1";
pub const XAI_BASE_URL: &str = "https://api.x.ai/v1";
pub const REPLICATE_BASE_URL: &str = "https://api.replicate.com/v1";
pub const VENICE_BASE_URL: &str = "https://api.venice.ai/api/v1";
pub const NVIDIA_NIM_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";

// ── Chinese providers ─────────────────────────────────────────────
pub const QWEN_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
/// Global endpoint. For China mainland, override via `[provider_urls] minimax = "https://api.minimaxi.com/v1"`.
pub const MINIMAX_BASE_URL: &str = "https://api.minimax.io/v1";
pub const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
pub const ZHIPU_CODING_BASE_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4";
/// Zhipu/GLM Anthropic-compatible endpoint (uses Anthropic Messages API format).
pub const ZHIPU_ANTHROPIC_BASE_URL: &str = "https://open.bigmodel.cn/api/anthropic";
/// Z.AI domain aliases (same API, different domain).
pub const ZAI_BASE_URL: &str = "https://api.z.ai/api/paas/v4";
pub const ZAI_CODING_BASE_URL: &str = "https://api.z.ai/api/coding/paas/v4";
pub const MOONSHOT_BASE_URL: &str = "https://api.moonshot.ai/v1";
pub const KIMI_CODING_BASE_URL: &str = "https://api.kimi.com/coding";
pub const QIANFAN_BASE_URL: &str = "https://qianfan.baidubce.com/v2";
pub const VOLCENGINE_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
pub const VOLCENGINE_CODING_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/coding/v3";

// ── Chutes.ai ────────────────────────────────────────────────────
pub const CHUTES_BASE_URL: &str = "https://llm.chutes.ai/v1";

// ── Azure OpenAI ────────────────────────────────────────────────────
/// Azure OpenAI requires a per-resource URL. Users must set their own via
/// `base_url` or `[provider_urls] azure = "https://{resource}.openai.azure.com/openai/deployments"`.
/// This constant is intentionally empty — it is never used as a default.
pub const AZURE_OPENAI_BASE_URL: &str = "";

// ── AWS Bedrock ───────────────────────────────────────────────────
pub const BEDROCK_BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

/// Provider authentication status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// API key is present in the environment.
    Configured,
    /// API key is missing.
    #[default]
    Missing,
    /// No API key required (local providers).
    NotRequired,
}

impl fmt::Display for AuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthStatus::Configured => write!(f, "configured"),
            AuthStatus::Missing => write!(f, "missing"),
            AuthStatus::NotRequired => write!(f, "not_required"),
        }
    }
}

/// A single model entry in the catalog.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    /// Canonical model identifier (e.g. "claude-sonnet-4-20250514").
    pub id: String,
    /// Provider identifier (e.g. "anthropic").
    pub provider: String,
    /// Aliases for this model (e.g. ["sonnet", "claude-sonnet"]).
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Provider metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Provider identifier (e.g. "anthropic").
    pub id: String,
    /// Human-readable display name (e.g. "Anthropic").
    pub display_name: String,
    /// Environment variable name for the API key.
    pub api_key_env: String,
    /// Default base URL.
    pub base_url: String,
    /// Whether an API key is required (false for local providers).
    pub key_required: bool,
    /// Runtime-detected authentication status.
    pub auth_status: AuthStatus,
    /// Number of models from this provider in the catalog.
    pub model_count: usize,
}

impl Default for ProviderInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            api_key_env: String::new(),
            base_url: String::new(),
            key_required: true,
            auth_status: AuthStatus::default(),
            model_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_status_display() {
        assert_eq!(AuthStatus::Configured.to_string(), "configured");
        assert_eq!(AuthStatus::Missing.to_string(), "missing");
        assert_eq!(AuthStatus::NotRequired.to_string(), "not_required");
    }

    #[test]
    fn test_auth_status_default() {
        assert_eq!(AuthStatus::default(), AuthStatus::Missing);
    }

    #[test]
    fn test_model_catalog_entry_default() {
        let entry = ModelCatalogEntry::default();
        assert!(entry.id.is_empty());
        assert!(entry.provider.is_empty());
        assert!(entry.aliases.is_empty());
    }

    #[test]
    fn test_provider_info_default() {
        let info = ProviderInfo::default();
        assert!(info.id.is_empty());
        assert!(info.key_required);
        assert_eq!(info.auth_status, AuthStatus::Missing);
    }

    #[test]
    fn test_auth_status_serde_roundtrip() {
        let status = AuthStatus::Configured;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"configured\"");
        let parsed: AuthStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn test_model_entry_serde_roundtrip() {
        let entry = ModelCatalogEntry {
            id: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            aliases: vec!["sonnet".to_string(), "claude-sonnet".to_string()],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ModelCatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.aliases.len(), 2);
    }

    #[test]
    fn test_provider_info_serde_roundtrip() {
        let info = ProviderInfo {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            key_required: true,
            auth_status: AuthStatus::Configured,
            model_count: 3,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "anthropic");
        assert_eq!(parsed.auth_status, AuthStatus::Configured);
        assert_eq!(parsed.model_count, 3);
    }

    #[test]
    fn test_azure_openai_base_url_empty() {
        assert!(AZURE_OPENAI_BASE_URL.is_empty());
    }
}
