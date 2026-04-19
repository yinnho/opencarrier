//! Model catalog — registry of models with auth detection.
//!
//! Models and providers are populated dynamically via Brain configuration,
//! provider URL overrides, or local provider discovery (Ollama, vLLM, etc.).

use opencarrier_types::model_catalog::{AuthStatus, ModelCatalogEntry, ProviderInfo};
use std::collections::HashMap;

/// The model catalog — registry of all known models and providers.
pub struct ModelCatalog {
    models: Vec<ModelCatalogEntry>,
    aliases: HashMap<String, String>,
    providers: Vec<ProviderInfo>,
}

impl ModelCatalog {
    /// Create a new empty catalog.
    ///
    /// Models and providers are populated dynamically via Brain configuration,
    /// provider URL overrides, or local provider discovery.
    pub fn new() -> Self {
        Self {
            models: vec![],
            aliases: HashMap::new(),
            providers: vec![],
        }
    }

    /// Detect which providers have API keys configured.
    ///
    /// Checks `std::env::var()` for each provider's API key env var.
    /// Only checks presence — never reads or stores the actual secret.
    pub fn detect_auth(&mut self) {
        for provider in &mut self.providers {
            // Claude Code is special: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if provider.id == "claude-code" {
                provider.auth_status = if crate::drivers::claude_code::claude_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }
            if provider.id == "qwen-code" {
                provider.auth_status = if crate::drivers::qwen_code::qwen_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }

            if !provider.key_required {
                provider.auth_status = AuthStatus::NotRequired;
                continue;
            }

            // Primary: check the provider's declared env var
            let has_key = std::env::var(&provider.api_key_env).is_ok();

            // Secondary: provider-specific fallback auth
            let has_fallback = match provider.id.as_str() {
                "gemini" => std::env::var("GOOGLE_API_KEY").is_ok(),
                "codex" => {
                    std::env::var("OPENAI_API_KEY").is_ok() || read_codex_credential().is_some()
                }
                // claude-code is handled above (before key_required check)
                _ => false,
            };

            provider.auth_status = if has_key || has_fallback {
                AuthStatus::Configured
            } else {
                AuthStatus::Missing
            };
        }
    }

    /// List all models in the catalog.
    pub fn list_models(&self) -> &[ModelCatalogEntry] {
        &self.models
    }

    /// Find a model by its canonical ID or by alias.
    pub fn find_model(&self, id_or_alias: &str) -> Option<&ModelCatalogEntry> {
        let lower = id_or_alias.to_lowercase();
        // Direct ID match first
        if let Some(entry) = self.models.iter().find(|m| m.id.to_lowercase() == lower) {
            return Some(entry);
        }
        // Alias resolution
        if let Some(canonical) = self.aliases.get(&lower) {
            return self.models.iter().find(|m| m.id == *canonical);
        }
        None
    }

    /// Resolve an alias to a canonical model ID, or None if not an alias.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(&alias.to_lowercase()).map(|s| s.as_str())
    }

    /// List all providers.
    pub fn list_providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    /// Get a provider by ID.
    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderInfo> {
        self.providers.iter().find(|p| p.id == provider_id)
    }

    /// List all alias mappings.
    pub fn list_aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    /// Set a custom base URL for a provider, overriding the default.
    ///
    /// Returns `true` if the provider was found or added.
    pub fn set_provider_url(&mut self, provider: &str, url: &str) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.base_url = url.to_string();
            true
        } else {
            // Custom provider — add a new entry
            let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
            self.providers.push(ProviderInfo {
                id: provider.to_string(),
                display_name: provider.to_string(),
                api_key_env: env_var,
                base_url: url.to_string(),
                key_required: true,
                auth_status: AuthStatus::Missing,
                model_count: 0,
            });
            // Re-detect auth for the newly added provider
            self.detect_auth();
            true
        }
    }

    /// Apply a batch of provider URL overrides from config.
    ///
    /// Each entry maps a provider ID to a custom base URL.
    /// Unknown providers are automatically added as custom OpenAI-compatible entries.
    pub fn apply_url_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (provider, url) in overrides {
            if self.set_provider_url(provider, url) {
                if let Some(p) = self.providers.iter_mut().find(|p| p.id == *provider) {
                    if p.auth_status == AuthStatus::Missing {
                        p.auth_status = AuthStatus::Configured;
                    }
                }
            }
        }
    }

    /// Merge dynamically discovered models from a local provider.
    ///
    /// Adds models not already in the catalog.
    /// Also updates the provider's `model_count`.
    pub fn merge_discovered_models(&mut self, provider: &str, model_ids: &[String]) {
        let mut existing_ids: std::collections::HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider)
            .map(|m| m.id.to_lowercase())
            .collect();

        let mut added = 0usize;
        for id in model_ids {
            let lower = id.to_lowercase();
            if existing_ids.contains(&lower) {
                continue;
            }
            self.models.push(ModelCatalogEntry {
                id: id.clone(),
                provider: provider.to_string(),
                aliases: Vec::new(),
            });
            existing_ids.insert(lower);
            added += 1;
        }

        // Update model count on the provider
        if added > 0 {
            if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
                p.model_count = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider)
                    .count();
            }
        }
    }

    /// Add a custom model at runtime.
    ///
    /// Returns `true` if the model was added, `false` if a model with the same
    /// ID **and** provider already exists (case-insensitive).
    pub fn add_custom_model(&mut self, entry: ModelCatalogEntry) -> bool {
        let lower_id = entry.id.to_lowercase();
        let lower_provider = entry.provider.to_lowercase();
        if self
            .models
            .iter()
            .any(|m| m.id.to_lowercase() == lower_id && m.provider.to_lowercase() == lower_provider)
        {
            return false;
        }
        let provider = entry.provider.clone();
        // Register aliases from the entry
        for alias in &entry.aliases {
            let lower = alias.to_lowercase();
            self.aliases.entry(lower).or_insert_with(|| entry.id.clone());
        }
        self.models.push(entry);

        // Update provider model count
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.model_count = self
                .models
                .iter()
                .filter(|m| m.provider == provider)
                .count();
        }
        true
    }

    /// Remove a model by ID.
    ///
    /// Returns `true` if removed.
    pub fn remove_custom_model(&mut self, model_id: &str) -> bool {
        let lower = model_id.to_lowercase();
        let before = self.models.len();
        self.models.retain(|m| m.id.to_lowercase() != lower);
        self.models.len() < before
    }

    /// Load custom models from a JSON file.
    ///
    /// Merges them into the catalog. Skips models that already exist.
    pub fn load_custom_models(&mut self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        let Ok(data) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(entries) = serde_json::from_str::<Vec<ModelCatalogEntry>>(&data) else {
            return;
        };
        for entry in entries {
            self.add_custom_model(entry);
        }
    }

    /// Save all models to a JSON file.
    pub fn save_custom_models(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.models)
            .map_err(|e| format!("Failed to serialize models: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write models file: {e}"))?;
        Ok(())
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Read an OpenAI API key from the Codex CLI credential file.
///
/// Checks `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
/// Returns `Some(api_key)` if the file exists and contains a valid, non-expired token.
pub fn read_codex_credential() -> Option<String> {
    let codex_home = std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
        })?;

    let auth_path = codex_home.join("auth.json");
    let content = std::fs::read_to_string(&auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check expiry if present
    if let Some(expires_at) = parsed.get("expires_at").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if now >= expires_at {
            return None; // Expired
        }
    }

    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_catalog_is_empty() {
        let catalog = ModelCatalog::new();
        assert!(catalog.list_models().is_empty());
        assert!(catalog.list_providers().is_empty());
        assert!(catalog.list_aliases().is_empty());
    }

    #[test]
    fn test_add_and_find_model() {
        let mut catalog = ModelCatalog::new();
        let entry = ModelCatalogEntry {
            id: "test-model".to_string(),
            provider: "test-provider".to_string(),
            aliases: vec!["tm".to_string()],
        };
        assert!(catalog.add_custom_model(entry));
        assert!(catalog.find_model("test-model").is_some());
        assert!(catalog.find_model("tm").is_some());
        assert!(catalog.find_model("nonexistent").is_none());
    }

    #[test]
    fn test_add_duplicate_model() {
        let mut catalog = ModelCatalog::new();
        let entry = ModelCatalogEntry {
            id: "test-model".to_string(),
            provider: "test-provider".to_string(),
            aliases: vec![],
        };
        assert!(catalog.add_custom_model(entry));
        let dup = ModelCatalogEntry {
            id: "test-model".to_string(),
            provider: "test-provider".to_string(),
            aliases: vec![],
        };
        assert!(!catalog.add_custom_model(dup));
    }

    #[test]
    fn test_remove_model() {
        let mut catalog = ModelCatalog::new();
        catalog.add_custom_model(ModelCatalogEntry {
            id: "to-remove".to_string(),
            provider: "test".to_string(),
            aliases: vec![],
        });
        assert!(catalog.remove_custom_model("to-remove"));
        assert!(!catalog.remove_custom_model("to-remove")); // Already gone
    }

    #[test]
    fn test_alias_resolution() {
        let mut catalog = ModelCatalog::new();
        catalog.add_custom_model(ModelCatalogEntry {
            id: "claude-sonnet-4".to_string(),
            provider: "anthropic".to_string(),
            aliases: vec!["sonnet".to_string()],
        });
        assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4"));
        assert_eq!(catalog.resolve_alias("nonexistent"), None);
    }

    #[test]
    fn test_merge_discovered_models() {
        let mut catalog = ModelCatalog::new();
        catalog.providers.push(ProviderInfo {
            id: "ollama".to_string(),
            display_name: "Ollama".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: "http://localhost:11434/v1".to_string(),
            key_required: false,
            auth_status: AuthStatus::NotRequired,
            model_count: 0,
        });
        catalog.merge_discovered_models(
            "ollama",
            &[
                "llama3".to_string(),
                "mistral".to_string(),
                "llama3".to_string(), // duplicate — should be skipped
            ],
        );
        assert_eq!(catalog.list_models().len(), 2);
        assert!(catalog.find_model("llama3").is_some());
        assert!(catalog.find_model("mistral").is_some());
    }

    #[test]
    fn test_save_and_load_custom_models() {
        let dir = std::env::temp_dir().join("opencarrier_test_models");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom_models.json");

        let mut catalog = ModelCatalog::new();
        catalog.add_custom_model(ModelCatalogEntry {
            id: "model-a".to_string(),
            provider: "test".to_string(),
            aliases: vec!["a".to_string()],
        });
        catalog.save_custom_models(&path).unwrap();

        let mut catalog2 = ModelCatalog::new();
        catalog2.load_custom_models(&path);
        assert!(catalog2.find_model("model-a").is_some());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_provider_url_adds_new() {
        let mut catalog = ModelCatalog::new();
        assert!(catalog.set_provider_url("my-proxy", "http://localhost:8080/v1"));
        let provider = catalog.get_provider("my-proxy").unwrap();
        assert_eq!(provider.id, "my-proxy");
        assert_eq!(provider.base_url, "http://localhost:8080/v1");
    }

    #[test]
    fn test_apply_url_overrides() {
        let mut catalog = ModelCatalog::new();
        let mut overrides = HashMap::new();
        overrides.insert("ollama".to_string(), "http://192.168.1.100:11434/v1".to_string());
        catalog.apply_url_overrides(&overrides);
        let provider = catalog.get_provider("ollama").unwrap();
        assert_eq!(provider.base_url, "http://192.168.1.100:11434/v1");
    }
}
