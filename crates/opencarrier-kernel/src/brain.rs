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
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use opencarrier_runtime::llm_driver::{Brain as BrainTrait, DriverConfig, LlmDriver};
use opencarrier_runtime::drivers;
use opencarrier_types::brain::{ApiFormat, BrainConfig, BrainStatus, EndpointConfig, EndpointHealth, EndpointReport, ModalityInfo, ResolvedEndpoint};
use tracing::{info, warn};
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Per-endpoint health tracker (lock-free atomics)
// ---------------------------------------------------------------------------

/// Consecutive failures before circuit opens (endpoint is taken out of rotation).
const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
/// How long to wait before allowing a probe request (half-open state).
const CIRCUIT_BREAKER_COOLDOWN_MS: u64 = 60_000; // 60s

/// Thread-safe health tracker for a single endpoint.
struct EndpointTracker {
    success_count: AtomicU64,
    failure_count: AtomicU64,
    total_latency_ms: AtomicU64,
    latency_count: AtomicU64,
    consecutive_failures: AtomicU32,
    /// Timestamp (ms since epoch) of the last failure. Used for circuit-breaker cooldown.
    last_failure_at: AtomicU64,
}

impl EndpointTracker {
    fn new() -> Self {
        Self {
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            total_latency_ms: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
            consecutive_failures: AtomicU32::new(0),
            last_failure_at: AtomicU64::new(0),
        }
    }

    fn record_success(&self, latency_ms: u64) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        if latency_ms > 0 {
            self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
            self.latency_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_failure(&self, latency_ms: u64) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        if latency_ms > 0 {
            self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
            self.latency_count.fetch_add(1, Ordering::Relaxed);
        }
        self.last_failure_at.store(now_ms(), Ordering::Relaxed);
    }

    /// Check if the circuit is open (endpoint should be skipped).
    /// Returns true if the endpoint is available for requests.
    fn is_available(&self) -> bool {
        let consec = self.consecutive_failures.load(Ordering::Relaxed);
        if consec < CIRCUIT_BREAKER_THRESHOLD {
            return true;
        }
        // Circuit is open — check if cooldown has passed (half-open)
        let last = self.last_failure_at.load(Ordering::Relaxed);
        let elapsed = now_ms().saturating_sub(last);
        elapsed >= CIRCUIT_BREAKER_COOLDOWN_MS
    }

    fn snapshot(&self) -> EndpointSnapshot {
        let success = self.success_count.load(Ordering::Relaxed);
        let failure = self.failure_count.load(Ordering::Relaxed);
        let total_lat = self.total_latency_ms.load(Ordering::Relaxed);
        let lat_count = self.latency_count.load(Ordering::Relaxed);
        let avg = if lat_count > 0 { total_lat / lat_count } else { 0 };
        let consec = self.consecutive_failures.load(Ordering::Relaxed);
        let circuit_open = consec >= CIRCUIT_BREAKER_THRESHOLD && !self.is_available();
        EndpointSnapshot { success, failure, avg_latency: avg, consecutive_failures: consec, circuit_open }
    }
}

struct EndpointSnapshot {
    success: u64,
    failure: u64,
    avg_latency: u64,
    consecutive_failures: u32,
    circuit_open: bool,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Brain
// ---------------------------------------------------------------------------

/// The carrier's brain — manages all LLM drivers and routes by modality.
pub struct Brain {
    config: BrainConfig,
    /// Pre-created drivers, keyed by endpoint name.
    drivers: HashMap<String, Arc<dyn LlmDriver>>,
    /// Per-endpoint health tracking. Thread-safe for concurrent report() calls.
    health: DashMap<String, EndpointTracker>,
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

        Ok(Self { config, drivers, health: DashMap::new() })
    }

    // ── New query interface ─────────────────────────────────────

    /// List all available modalities with descriptions.
    pub fn list_modalities(&self) -> Vec<ModalityInfo> {
        self.config.modalities.iter().map(|(name, mc)| {
            ModalityInfo {
                name: name.clone(),
                description: mc.description.clone(),
                primary_endpoint: mc.primary.clone(),
                fallback_count: mc.fallbacks.len(),
            }
        }).collect()
    }

    /// Get the ordered list of resolved endpoints for a modality.
    /// Returns primary first, then fallbacks. Filters out endpoints
    /// with no live driver **or circuit-broken** (too many consecutive failures).
    pub fn endpoints_for(&self, modality: &str) -> Vec<ResolvedEndpoint> {
        let mod_config = self.config.modalities.get(modality)
            .or_else(|| self.config.modalities.get(&self.config.default_modality));

        let Some(mod_config) = mod_config else {
            return vec![];
        };

        let mut chain = vec![mod_config.primary.clone()];
        chain.extend(mod_config.fallbacks.iter().cloned());

        chain.into_iter()
            .filter_map(|name| {
                let endpoint = self.config.endpoints.get(&name)?;
                // Only include endpoints with live drivers
                if !self.drivers.contains_key(&name) {
                    return None;
                }
                // Circuit-breaker: skip endpoints with too many consecutive failures
                if let Some(tracker) = self.health.get(&name) {
                    if !tracker.is_available() {
                        warn!(
                            endpoint = %name,
                            consecutive = tracker.consecutive_failures.load(Ordering::Relaxed),
                            "Endpoint circuit-broken, skipping"
                        );
                        return None;
                    }
                }
                Some(ResolvedEndpoint {
                    id: name,
                    model: endpoint.model.clone(),
                    provider: endpoint.provider.clone(),
                })
            })
            .collect()
    }

    /// Get a driver for a specific endpoint. Returns None if no driver.
    pub fn driver_for_endpoint(&self, endpoint_id: &str) -> Option<Arc<dyn LlmDriver>> {
        self.drivers.get(endpoint_id).cloned()
    }

    /// Report the result of an endpoint call. Non-blocking.
    pub fn report(&self, report: EndpointReport) {
        let tracker = self.health
            .entry(report.endpoint_id)
            .or_insert_with(EndpointTracker::new);

        if report.success {
            tracker.record_success(report.latency_ms);
        } else {
            tracker.record_failure(report.latency_ms);
        }
    }

    /// Get current Brain status snapshot.
    pub fn status(&self) -> BrainStatus {
        let modalities = self.list_modalities();

        let endpoints: Vec<EndpointHealth> = self.config.endpoints.iter()
            .map(|(name, ep)| {
                let snap = self.health
                    .get(name)
                    .map(|t| t.snapshot())
                    .unwrap_or_else(|| EndpointSnapshot {
                        success: 0, failure: 0, avg_latency: 0,
                        consecutive_failures: 0, circuit_open: false,
                    });

                EndpointHealth {
                    endpoint: name.clone(),
                    provider: ep.provider.clone(),
                    model: ep.model.clone(),
                    driver_ready: self.drivers.contains_key(name),
                    success_count: snap.success,
                    failure_count: snap.failure,
                    avg_latency_ms: snap.avg_latency,
                    consecutive_failures: snap.consecutive_failures,
                    circuit_open: snap.circuit_open,
                }
            })
            .collect();

        let drivers_ready = self.drivers.len();

        BrainStatus { modalities, endpoints, drivers_ready }
    }

    /// Resolve credentials for a provider (for skill credential injection).
    /// Reads all env vars declared in the provider config and returns them
    /// as a ProviderCredentials struct ready for injection into skill subprocesses.
    pub fn credentials_for(&self, provider: &str) -> Option<opencarrier_types::brain::ProviderCredentials> {
        let config = self.config.providers.get(provider)?;
        let mut env_vars = HashMap::new();

        // Legacy: api_key_env
        if !config.api_key_env.is_empty() {
            if let Ok(val) = std::env::var(&config.api_key_env) {
                env_vars.insert(config.api_key_env.clone(), val);
            }
        }

        // New: params (each value is an env var name)
        for env_name in config.params.values() {
            if let Ok(val) = std::env::var(env_name) {
                env_vars.insert(env_name.clone(), val);
            }
        }

        Some(opencarrier_types::brain::ProviderCredentials {
            provider_name: provider.to_string(),
            env_vars,
        })
    }

    // ── Legacy methods ─────────────────────────────────────────

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

        // Pass the provider name to create_driver so it can look up
        // provider-specific defaults (e.g., ollama = key_required: false).
        // For known format-only providers (anthropic/gemini), we still map
        // via the format to pick the right driver type.
        let driver_provider = match endpoint.format {
            ApiFormat::OpenAI => {
                // For OpenAI-compatible format, pass the actual provider name.
                // This lets ollama/vllm/lmstudio work without API keys,
                // while providers like openai/groq/deepseek still require keys.
                endpoint.provider.as_str()
            }
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

/// Implement the runtime Brain trait so agent_loop can use Brain methods.
#[async_trait]
impl BrainTrait for Brain {
    fn list_modalities(&self) -> Vec<ModalityInfo> {
        Brain::list_modalities(self)
    }

    fn endpoints_for(&self, modality: &str) -> Vec<ResolvedEndpoint> {
        Brain::endpoints_for(self, modality)
    }

    fn driver_for_endpoint(&self, endpoint_id: &str) -> Option<Arc<dyn LlmDriver>> {
        Brain::driver_for_endpoint(self, endpoint_id)
    }

    fn report(&self, report: EndpointReport) {
        Brain::report(self, report)
    }

    fn status(&self) -> BrainStatus {
        Brain::status(self)
    }

    fn credentials_for(&self, provider: &str) -> Option<opencarrier_types::brain::ProviderCredentials> {
        Brain::credentials_for(self, provider)
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
