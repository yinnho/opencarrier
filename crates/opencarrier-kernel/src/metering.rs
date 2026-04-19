//! Metering engine — tracks LLM token usage.

use opencarrier_memory::usage::{UsageRecord, UsageStore, UsageSummary};
use opencarrier_types::agent::AgentId;
use opencarrier_types::error::OpenCarrierResult;
use std::sync::Arc;

/// The metering engine tracks token usage.
pub struct MeteringEngine {
    /// Persistent usage store (SQLite-backed).
    store: Arc<UsageStore>,
}

impl MeteringEngine {
    /// Create a new metering engine with the given usage store.
    pub fn new(store: Arc<UsageStore>) -> Self {
        Self { store }
    }

    /// Record a usage event (persists to SQLite).
    pub fn record(&self, record: &UsageRecord) -> OpenCarrierResult<()> {
        self.store.record(record)
    }

    /// Get a usage summary, optionally filtered by agent.
    pub fn get_summary(&self, agent_id: Option<AgentId>) -> OpenCarrierResult<UsageSummary> {
        self.store.query_summary(agent_id)
    }

    /// Get usage grouped by model.
    pub fn get_by_model(&self) -> OpenCarrierResult<Vec<opencarrier_memory::usage::ModelUsage>> {
        self.store.query_by_model()
    }

    /// Clean up old usage records.
    pub fn cleanup(&self, days: u32) -> OpenCarrierResult<usize> {
        self.store.cleanup_old(days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencarrier_memory::MemorySubstrate;

    fn setup() -> MeteringEngine {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = Arc::new(UsageStore::new(substrate.usage_conn()));
        MeteringEngine::new(store)
    }

    #[test]
    fn test_record_and_get_summary() {
        let engine = setup();
        let agent_id = AgentId::new();

        engine
            .record(&UsageRecord {
                agent_id,
                model: "test-model".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                tool_calls: 3,
            })
            .unwrap();

        let summary = engine.get_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 1);
        assert_eq!(summary.total_input_tokens, 500);
    }
}
