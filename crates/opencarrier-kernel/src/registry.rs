//! Agent registry — tracks all agents, their state, and indexes.

use dashmap::DashMap;
use opencarrier_types::agent::{AgentEntry, AgentId, AgentMode, AgentState};
use opencarrier_types::error::{OpenCarrierError, OpenCarrierResult};

/// Registry of all agents in the kernel.
pub struct AgentRegistry {
    /// Primary index: agent ID → entry.
    agents: DashMap<AgentId, AgentEntry>,
    /// Name index: (tenant_id, agent_name) → agent ID.
    /// Per-tenant uniqueness: same name allowed across different tenants.
    name_index: DashMap<(String, String), AgentId>,
    /// Tag index: tag → list of agent IDs.
    tag_index: DashMap<String, Vec<AgentId>>,
    /// Tenant index: tenant_id → list of agent IDs.
    tenant_index: DashMap<String, Vec<AgentId>>,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
            tenant_index: DashMap::new(),
        }
    }

    /// Register a new agent.
    /// Name uniqueness is per-tenant: same name allowed across different tenants.
    pub fn register(&self, entry: AgentEntry) -> OpenCarrierResult<()> {
        let name_key = (entry.tenant_id.clone(), entry.name.clone());
        if self.name_index.contains_key(&name_key) {
            return Err(OpenCarrierError::AgentAlreadyExists(entry.name.clone()));
        }
        let id = entry.id;
        self.name_index.insert(name_key, id);
        for tag in &entry.tags {
            self.tag_index.entry(tag.clone()).or_default().push(id);
        }
        self.tenant_index
            .entry(entry.tenant_id.clone())
            .or_default()
            .push(id);
        self.agents.insert(id, entry);
        Ok(())
    }

    /// Get an agent entry by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentEntry> {
        self.agents.get(&id).map(|e| e.value().clone())
    }

    /// Find an agent by name within a specific tenant scope.
    pub fn find_by_name_and_tenant(&self, name: &str, tenant_id: &str) -> Option<AgentEntry> {
        let key = (tenant_id.to_string(), name.to_string());
        self.name_index
            .get(&key)
            .and_then(|id| self.agents.get(id.value()).map(|e| e.value().clone()))
    }

    /// Find an agent by name (global, returns first match).
    /// Prefer `find_by_name_and_tenant` for tenant-scoped lookups.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        for entry in self.name_index.iter() {
            if entry.key().1 == name {
                let id = entry.value();
                return self.agents.get(id).map(|e| e.value().clone());
            }
        }
        None
    }

    /// Check if an agent with the given name exists under a specific tenant.
    pub fn exists_in_tenant(&self, name: &str, tenant_id: &str) -> bool {
        let key = (tenant_id.to_string(), name.to_string());
        self.name_index.contains_key(&key)
    }

    /// Update agent state.
    pub fn set_state(&self, id: AgentId, state: AgentState) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.state = state;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update agent operational mode.
    pub fn set_mode(&self, id: AgentId, mode: AgentMode) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.mode = mode;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Remove an agent from the registry.
    pub fn remove(&self, id: AgentId) -> OpenCarrierResult<AgentEntry> {
        let (_, entry) = self
            .agents
            .remove(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        let name_key = (entry.tenant_id.clone(), entry.name.clone());
        self.name_index.remove(&name_key);
        for tag in &entry.tags {
            if let Some(mut ids) = self.tag_index.get_mut(tag) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        if let Some(mut ids) = self.tenant_index.get_mut(&entry.tenant_id) {
            ids.retain(|&agent_id| agent_id != id);
        }
        Ok(entry)
    }

    /// List all agents.
    pub fn list(&self) -> Vec<AgentEntry> {
        self.agents.iter().map(|e| e.value().clone()).collect()
    }

    /// List agents belonging to a specific tenant.
    pub fn list_by_tenant(&self, tenant_id: &str) -> Vec<AgentEntry> {
        self.tenant_index
            .get(tenant_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.agents.get(id).map(|e| e.value().clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Add a child agent ID to a parent's children list.
    pub fn add_child(&self, parent_id: AgentId, child_id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&parent_id) {
            entry.children.push(child_id);
        }
    }

    /// Update the tenant_id on an agent entry and re-index.
    pub fn set_tenant_id(&self, agent_id: AgentId, tenant_id: String) {
        if let Some(mut entry) = self.agents.get_mut(&agent_id) {
            let old_tid = entry.tenant_id.clone();
            entry.tenant_id = tenant_id.clone();
            // Remove from old tenant index
            if let Some(mut ids) = self.tenant_index.get_mut(&old_tid) {
                ids.retain(|id| *id != agent_id);
            }
            // Add to new tenant index
            self.tenant_index
                .entry(tenant_id)
                .or_default()
                .push(agent_id);
        }
    }

    /// Count of registered agents.
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Update an agent's session ID (for session reset).
    pub fn update_session_id(
        &self,
        id: AgentId,
        new_session_id: opencarrier_types::agent::SessionId,
    ) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.session_id = new_session_id;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's workspace path.
    pub fn update_workspace(
        &self,
        id: AgentId,
        workspace: Option<std::path::PathBuf>,
    ) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.workspace = workspace;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's visual identity (emoji, avatar, color).
    pub fn update_identity(
        &self,
        id: AgentId,
        identity: opencarrier_types::agent::AgentIdentity,
    ) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.identity = identity;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's modality.
    pub fn update_modality(&self, id: AgentId, modality: String) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.modality = modality;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's skill allowlist.
    pub fn update_skills(&self, id: AgentId, skills: Vec<String>) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.skills = skills;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's MCP server allowlist.
    pub fn update_mcp_servers(&self, id: AgentId, servers: Vec<String>) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.mcp_servers = servers;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's tool allowlist and blocklist.
    pub fn update_tool_filters(
        &self,
        id: AgentId,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        if let Some(al) = allowlist {
            entry.manifest.tool_allowlist = al;
        }
        if let Some(bl) = blocklist {
            entry.manifest.tool_blocklist = bl;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's system prompt (hot-swap, takes effect on next message).
    pub fn update_system_prompt(&self, id: AgentId, new_prompt: String) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.system_prompt = new_prompt;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's name (also updates the name index).
    pub fn update_name(&self, id: AgentId, new_name: String) -> OpenCarrierResult<()> {
        let entry = self
            .agents
            .get(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        let old_name = entry.name.clone();
        let tenant_id = entry.tenant_id.clone();
        drop(entry);

        let new_key = (tenant_id.clone(), new_name.clone());
        if let Some(existing_id) = self.name_index.get(&new_key).as_deref().copied() {
            if existing_id != id {
                return Err(OpenCarrierError::AgentAlreadyExists(new_name));
            }
            return Ok(());
        }
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.name = new_name.clone();
        entry.manifest.name = new_name.clone();
        entry.last_active = chrono::Utc::now();
        drop(entry);
        let old_key = (tenant_id, old_name);
        self.name_index.remove(&old_key);
        self.name_index.insert(new_key, id);
        Ok(())
    }

    /// Update an agent's description.
    pub fn update_description(&self, id: AgentId, new_desc: String) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.manifest.description = new_desc;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's resource limits.
    pub fn update_resources(
        &self,
        id: AgentId,
        tokens_per_hour: Option<u64>,
    ) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        if let Some(v) = tokens_per_hour {
            entry.manifest.resources.max_llm_tokens_per_hour = v;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Mark an agent's onboarding as complete.
    pub fn mark_onboarding_complete(&self, id: AgentId) -> OpenCarrierResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenCarrierError::AgentNotFound(id.to_string()))?;
        entry.onboarding_completed = true;
        entry.onboarding_completed_at = Some(chrono::Utc::now());
        entry.last_active = chrono::Utc::now();
        Ok(())
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use opencarrier_types::agent::*;
    use std::collections::HashMap;

    fn test_entry(name: &str) -> AgentEntry {
        test_entry_with_tenant(name, "test-tenant")
    }

    fn test_entry_with_tenant(name: &str, tenant_id: &str) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            name: name.to_string(),
            manifest: AgentManifest {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                description: "test".to_string(),
                author: "test".to_string(),
                module: "test".to_string(),
                schedule: ScheduleMode::default(),
                model: ModelConfig::default(),
                resources: ResourceQuota::default(),
                priority: Priority::default(),
                capabilities: ManifestCapabilities::default(),
                profile: None,
                tools: HashMap::new(),
                skills: vec![],
                mcp_servers: vec![],
                metadata: HashMap::new(),
                tags: vec![],
                autonomous: None,
                workspace: None,
                generate_identity_files: true,
                exec_policy: None,
                tool_allowlist: vec![],
                tool_blocklist: vec![],
                clone_source: None,
                knowledge_files: vec![],
                plugins: vec![],
            },
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            tenant_id: tenant_id.to_string(),
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = AgentRegistry::new();
        let entry = test_entry("test-agent");
        let id = entry.id;
        registry.register(entry).unwrap();
        assert!(registry.get(id).is_some());
    }

    #[test]
    fn test_find_by_name() {
        let registry = AgentRegistry::new();
        let entry = test_entry("my-agent");
        registry.register(entry).unwrap();
        assert!(registry.find_by_name("my-agent").is_some());
    }

    #[test]
    fn test_duplicate_name_same_tenant() {
        let registry = AgentRegistry::new();
        registry
            .register(test_entry_with_tenant("dup", "t1"))
            .unwrap();
        assert!(registry
            .register(test_entry_with_tenant("dup", "t1"))
            .is_err());
    }

    #[test]
    fn test_same_name_different_tenant() {
        let registry = AgentRegistry::new();
        registry
            .register(test_entry_with_tenant("helper", "t1"))
            .unwrap();
        registry
            .register(test_entry_with_tenant("helper", "t2"))
            .unwrap();
        assert!(registry.find_by_name_and_tenant("helper", "t1").is_some());
        assert!(registry.find_by_name_and_tenant("helper", "t2").is_some());
    }

    #[test]
    fn test_remove() {
        let registry = AgentRegistry::new();
        let entry = test_entry("removable");
        let id = entry.id;
        registry.register(entry).unwrap();
        registry.remove(id).unwrap();
        assert!(registry.get(id).is_none());
    }
}
