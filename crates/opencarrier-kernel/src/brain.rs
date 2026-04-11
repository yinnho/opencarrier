//! Brain — the carrier's independent LLM brain.
//!
//! Three-layer architecture:
//! - **Provider**: identity + credentials
//! - **Endpoint**: complete callable unit (provider + model + base_url + format)
//! - **Modality**: task type → endpoint with fallback chain
//!
//! Drivers are pre-created and cached per endpoint at boot.

use std::collections::HashMap;
use std::sync::Arc;

use opencarrier_runtime::llm_driver::{Brain as BrainTrait, CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError};
use opencarrier_runtime::drivers;
use opencarrier_types::brain::{ApiFormat, BrainConfig, EndpointConfig};
use tracing::{debug, info, warn};
use async_trait::async_trait;

/// The carrier's brain — manages all LLM drivers and routes by modality.
pub struct Brain {
    config: BrainConfig,
    /// Pre-created drivers, keyed by endpoint name.
    drivers: HashMap<String, Arc<dyn LlmDriver>>,
}

impl Brain {
    /// Create a new Brain from config, pre-creating all endpoint drivers.
    pub fn new(config: BrainConfig) -> Result<Self, BrainError> {
        let mut drivers = HashMap::new();

        for (name, endpoint) in &config.endpoints {
            match Self::create_driver(name, endpoint, &config.providers) {
                Ok(driver) => {
                    info!(
                        endpoint = %name,
                        provider = %endpoint.provider,
                        model = %endpoint.model,
                        format = %endpoint.format,
                        "Brain endpoint ready"
                    );
                    drivers.insert(name.clone(), driver);
                }
                Err(e) => {
                    warn!(
                        endpoint = %name,
                        error = %e,
                        "Failed to create driver for endpoint — skipping"
                    );
                }
            }
        }

        if drivers.is_empty() {
            return Err(BrainError::NoEndpoints);
        }

        // Validate modalities reference existing endpoints
        for (mod_name, mod_config) in &config.modalities {
            if !drivers.contains_key(&mod_config.primary) {
                warn!(
                    modality = %mod_name,
                    primary = %mod_config.primary,
                    "Modality primary endpoint has no driver — will fail at runtime"
                );
            }
            for fb in &mod_config.fallbacks {
                if !drivers.contains_key(fb) {
                    warn!(
                        modality = %mod_name,
                        fallback = %fb,
                        "Modality fallback endpoint has no driver"
                    );
                }
            }
        }

        info!(
            endpoints = drivers.len(),
            modalities = config.modalities.len(),
            default_modality = %config.default_modality,
            "Brain initialized"
        );

        Ok(Self { config, drivers })
    }

    /// Core API — think with a given modality.
    ///
    /// Tries the primary endpoint, then fallbacks in order.
    pub async fn think(
        &self,
        modality: &str,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mod_config = self.config.modalities.get(modality)
            .or_else(|| self.config.modalities.get(&self.config.default_modality))
            .ok_or_else(|| LlmError::Api {
                status: 0,
                message: format!("Unknown modality '{}' and no default modality", modality),
            })?;

        // Build endpoint chain: primary + fallbacks
        let mut chain = vec![mod_config.primary.clone()];
        chain.extend(mod_config.fallbacks.iter().cloned());

        let mut last_error = None;
        for endpoint_name in &chain {
            if let Some(driver) = self.drivers.get(endpoint_name) {
                let model = self.model_for_endpoint(endpoint_name);
                let mut req = request.clone();
                req.model = model.to_string();

                debug!(endpoint = %endpoint_name, model = %model, "Trying endpoint");
                match driver.complete(req).await {
                    Ok(response) => {
                        debug!(endpoint = %endpoint_name, "Endpoint succeeded");
                        return Ok(response);
                    }
                    Err(e) => {
                        warn!(endpoint = %endpoint_name, error = %e, "Endpoint failed, trying next");
                        last_error = Some(e);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: format!("No driver available for modality '{}'", modality),
        }))
    }

    /// Get the model name for a given modality's primary endpoint.
    pub fn model_for(&self, modality: &str) -> &str {
        let mod_config = self.config.modalities.get(modality)
            .or_else(|| self.config.modalities.get(&self.config.default_modality));
        match mod_config {
            Some(mc) => self.model_for_endpoint(&mc.primary),
            None => "unknown",
        }
    }

    /// Get the default modality name.
    pub fn default_modality(&self) -> &str {
        &self.config.default_modality
    }

    /// List all available modalities.
    pub fn available_modalities(&self) -> Vec<&str> {
        self.config.modalities.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a modality is available.
    pub fn has_modality(&self, modality: &str) -> bool {
        self.config.modalities.contains_key(modality)
    }

    /// Get the underlying config (for dashboard API).
    pub fn config(&self) -> &BrainConfig {
        &self.config
    }

    /// Get the cached driver for a modality's primary endpoint.
    /// Returns None if no driver exists for the resolved endpoint.
    pub fn driver_for_modality(&self, modality: &str) -> Option<Arc<dyn LlmDriver>> {
        let mod_config = self.config.modalities.get(modality)
            .or_else(|| self.config.modalities.get(&self.config.default_modality))?;
        self.drivers.get(&mod_config.primary).cloned()
    }

    /// Get the endpoint names that have been successfully initialized (have drivers).
    pub fn ready_endpoints(&self) -> Vec<&str> {
        self.drivers.keys().map(|s| s.as_str()).collect()
    }

    // ── Internal helpers ──────────────────────────────────────

    fn model_for_endpoint(&self, endpoint_name: &str) -> &str {
        self.config.endpoints.get(endpoint_name)
            .map(|e| e.model.as_str())
            .unwrap_or("unknown")
    }

    fn create_driver(
        name: &str,
        endpoint: &EndpointConfig,
        providers: &HashMap<String, opencarrier_types::brain::ProviderConfig>,
    ) -> Result<Arc<dyn LlmDriver>, BrainError> {
        let provider_config = providers.get(&endpoint.provider)
            .ok_or_else(|| BrainError::ProviderNotFound {
                endpoint: name.to_string(),
                provider: endpoint.provider.clone(),
            })?;

        // Resolve API key from env var
        let api_key = if provider_config.api_key_env.is_empty() {
            None
        } else {
            std::env::var(&provider_config.api_key_env).ok()
        };

        // Map format to provider name for create_driver
        let driver_provider = match endpoint.format {
            ApiFormat::OpenAI => "openai",
            ApiFormat::Anthropic => "anthropic",
            ApiFormat::Gemini => "gemini",
        };

        let driver_config = DriverConfig {
            provider: driver_provider.to_string(),
            api_key,
            base_url: Some(endpoint.base_url.clone()),
            skip_permissions: true,
        };

        drivers::create_driver(&driver_config)
            .map_err(|e| BrainError::DriverCreation {
                endpoint: name.to_string(),
                error: e.to_string(),
            })
    }
}

/// Implement the runtime Brain trait so agent_loop can use brain.think().
#[async_trait]
impl BrainTrait for Brain {
    async fn think(
        &self,
        modality: &str,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        // Delegate to the internal think method
        Brain::think(self, modality, request).await
    }

    fn model_for(&self, modality: &str) -> &str {
        Brain::model_for(self, modality)
    }

    fn has_modality(&self, modality: &str) -> bool {
        Brain::has_modality(self, modality)
    }
}

/// Brain initialization errors.
#[derive(Debug)]
pub enum BrainError {
    /// No endpoints could be initialized.
    NoEndpoints,
    /// Referenced provider not found.
    ProviderNotFound { endpoint: String, provider: String },
    /// Driver creation failed.
    DriverCreation { endpoint: String, error: String },
}

impl std::fmt::Display for BrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrainError::NoEndpoints => write!(f, "No brain endpoints could be initialized"),
            BrainError::ProviderNotFound { endpoint, provider } => {
                write!(f, "Endpoint '{}' references unknown provider '{}'", endpoint, provider)
            }
            BrainError::DriverCreation { endpoint, error } => {
                write!(f, "Failed to create driver for '{}': {}", endpoint, error)
            }
        }
    }
}

impl std::error::Error for BrainError {}
