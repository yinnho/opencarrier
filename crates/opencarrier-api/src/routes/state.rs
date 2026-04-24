//! Shared application state for all route handlers.

use opencarrier_kernel::OpenCarrierKernel;
use std::sync::Arc;
use std::time::Instant;

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<OpenCarrierKernel>,
    pub started_at: Instant,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: opencarrier_runtime::provider_health::ProbeCache,
    /// Plugin manager (optional — only if plugins_dir is configured).
    #[allow(clippy::type_complexity)]
    pub plugin_manager: Option<Arc<tokio::sync::Mutex<opencarrier_runtime::plugin::PluginManager>>>,
}
