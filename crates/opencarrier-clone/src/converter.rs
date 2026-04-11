//! .agx CloneData → AgentManifest converter + workspace installer.

use std::path::Path;

use anyhow::{Context, Result};
use opencarrier_types::agent::{
    AgentManifest, CloneSource, ManifestCapabilities, ModelConfig, ResourceQuota,
};
use tracing::{debug, info};

use crate::loader::CloneData;

/// Convert parsed .agx data into an AgentManifest suitable for spawning.
///
/// Mapping:
/// - SOUL.md → workspace/SOUL.md (read by prompt_builder at runtime)
/// - system_prompt.md → workspace/system_prompt.md (read by prompt_builder at runtime)
/// - knowledge/ → workspace/data/knowledge/ (loaded on demand)
/// - skills/ → workspace/skills/ (activated on demand)
/// - manifest.model.system_prompt → empty (built dynamically from workspace files)
/// - skills → manifest.skills (names) + capabilities.tools (union of allowed_tools)
/// - profile name → manifest.name
/// - profile description → manifest.description
pub fn convert_to_manifest(data: &CloneData) -> AgentManifest {
    // Clone identity is stored as separate files in the workspace.
    // The prompt_builder reads SOUL.md, system_prompt.md, skills/, MEMORY.md,
    // and knowledge/ at runtime to dynamically build the system prompt.
    // No concatenation here — workspace IS the clone's identity.
    let system_prompt = String::new();

    // Collect skill names
    let skill_names: Vec<String> = data.skills.iter().map(|s| s.name.clone()).collect();

    // Collect all allowed_tools from skills
    let mut all_tools: Vec<String> = data.skills.iter()
        .flat_map(|s| s.allowed_tools.iter().cloned())
        .collect();
    all_tools.sort();
    all_tools.dedup();

    // Default tools for chat clones
    if all_tools.is_empty() {
        all_tools = vec![
            "file_read".into(),
            "file_list".into(),
            "memory_store".into(),
            "memory_recall".into(),
            "web_fetch".into(),
            "web_search".into(),
        ];
    }

    let clone_source = CloneSource {
        template_name: data.name.clone(),
        template_author: data.manifest.as_ref()
            .map(|m| m.author.clone())
            .unwrap_or_default(),
        installed_at: chrono::Utc::now().timestamp().to_string(),
        agx_version: data.manifest.as_ref()
            .map(|m| m.version.clone())
            .unwrap_or_else(|| "1".to_string()),
        hub_template_id: None,
    };

    let knowledge_files: Vec<String> = data.knowledge.keys().cloned().collect();

    AgentManifest {
        name: data.name.clone(),
        version: data.manifest.as_ref().map(|m| m.version.clone()).unwrap_or_else(|| "0.1.0".to_string()),
        description: if data.description.is_empty() {
            data.manifest.as_ref().map(|m| m.description.clone()).unwrap_or_default()
        } else {
            data.description.clone()
        },
        author: data.manifest.as_ref().map(|m| m.author.clone()).unwrap_or_default(),
        module: "builtin:chat".to_string(),
        schedule: opencarrier_types::agent::ScheduleMode::default(),
        model: ModelConfig {
            max_tokens: 8192,
            temperature: 0.7,
            system_prompt,
            modality: "chat".to_string(),
        },
        resources: ResourceQuota::default(),
        priority: opencarrier_types::agent::Priority::default(),
        capabilities: ManifestCapabilities {
            tools: all_tools,
            network: vec!["*".to_string()],
            memory_read: vec!["*".to_string()],
            memory_write: vec!["self.*".to_string()],
            ..Default::default()
        },
        skills: skill_names,
        tags: data.manifest.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
        clone_source: Some(clone_source),
        knowledge_files,
        generate_identity_files: false, // .agx already has its own identity files
        ..Default::default()
    }
}

/// Install clone data to a workspace directory.
///
/// Creates:
/// - agent.toml (the converted manifest)
/// - data/knowledge/*.md
/// - memory/index.md
/// - skills/<name>/SKILL.md + scripts/*.toml
pub fn install_clone_to_workspace(data: &CloneData, workspace: &Path) -> Result<()> {
    let manifest = convert_to_manifest(data);

    // Create workspace directory structure
    std::fs::create_dir_all(workspace)
        .with_context(|| format!("Failed to create workspace: {}", workspace.display()))?;

    let data_dir = workspace.join("data");
    let memory_dir = workspace.join("memory");
    let skills_dir = workspace.join("skills");
    let sessions_dir = workspace.join("sessions");
    let logs_dir = workspace.join("logs");
    let output_dir = workspace.join("output");

    for dir in &[&data_dir, &memory_dir, &skills_dir, &sessions_dir, &logs_dir, &output_dir] {
        std::fs::create_dir_all(dir)?;
    }

    // Write agent.toml
    let toml_str = toml::to_string_pretty(&manifest)
        .context("Failed to serialize AgentManifest to TOML")?;
    std::fs::write(workspace.join("agent.toml"), toml_str)?;
    info!("Wrote agent.toml to {}", workspace.display());

    // Write knowledge files
    let knowledge_dir = data_dir.join("knowledge");
    std::fs::create_dir_all(&knowledge_dir)?;
    for (name, content) in &data.knowledge {
        std::fs::write(knowledge_dir.join(name), content)?;
        debug!("Wrote knowledge file: {}", name);
    }

    // Write memory index
    if !data.memory_index.is_empty() {
        std::fs::write(memory_dir.join("index.md"), &data.memory_index)?;
    }

    // Write skills
    for skill in &data.skills {
        let skill_dir = skills_dir.join(&skill.name);
        std::fs::create_dir_all(&skill_dir)?;

        // Write SKILL.md
        let skill_md = format!(
            "---\nname: {}\nwhen_to_use: {}\nallowed_tools: {:?}\n---\n\n{}",
            skill.name, skill.when_to_use, skill.allowed_tools, skill.prompt
        );
        std::fs::write(skill_dir.join("SKILL.md"), skill_md)?;

        // Write scripts
        if !skill.scripts.is_empty() {
            let scripts_dir = skill_dir.join("scripts");
            std::fs::create_dir_all(&scripts_dir)?;
            for script in &skill.scripts {
                std::fs::write(scripts_dir.join(format!("{}.toml", script.name)), &script.toml_content)?;
            }
        }
    }

    // Write SOUL.md and system_prompt.md to workspace root (for reference)
    if !data.soul.is_empty() {
        std::fs::write(workspace.join("SOUL.md"), &data.soul)?;
    }
    if !data.system_prompt.is_empty() {
        std::fs::write(workspace.join("system_prompt.md"), &data.system_prompt)?;
    }
    if !data.profile.is_empty() {
        std::fs::write(workspace.join("profile.md"), &data.profile)?;
    }

    info!(
        "Installed clone '{}' to workspace: {} ({} knowledge files, {} skills)",
        data.name,
        workspace.display(),
        data.knowledge.len(),
        data.skills.len(),
    );

    Ok(())
}
