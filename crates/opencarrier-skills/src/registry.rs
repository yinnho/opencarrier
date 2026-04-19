//! Skill registry — tracks installed skills and their tools.

use crate::{InstalledSkill, SkillError, SkillManifest, SkillToolDef};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolve skills directory with yingheclient compatibility.
///
/// Priority:
/// 1. Environment variable `YINGHE_SKILLS_DIR` (yingheclient compat)
/// 2. Environment variable `OPENCARRIER_SKILLS_DIR` (OpenCarrier native)
/// 3. `skills` directory next to the executable (yingheclient compat)
/// 4. Default: `~/.opencarrier/skills` or `~/.yinghe/skills`
pub fn resolve_skills_dir() -> PathBuf {
    // 1. yingheclient environment variable
    if let Ok(dir) = std::env::var("YINGHE_SKILLS_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return path;
            }
        }
    }

    // 2. OpenCarrier environment variable
    if let Ok(dir) = std::env::var("OPENCARRIER_SKILLS_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return path;
            }
        }
    }

    // 3. Executable sibling directory
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let sibling = exe_dir.join("skills");
            if sibling.exists() {
                return sibling;
            }
        }
    }

    // 4. Default directories
    if let Some(home) = dirs::home_dir() {
        // Prefer opencarrier dir, but fall back to yinghe for compatibility
        let opencarrier_dir = home.join(".opencarrier").join("skills");
        if opencarrier_dir.exists() {
            return opencarrier_dir;
        }

        let yinghe_dir = home.join(".yinghe").join("skills");
        if yinghe_dir.exists() {
            return yinghe_dir;
        }

        // Return opencarrier dir even if it doesn't exist (will be created)
        return opencarrier_dir;
    }

    // Fallback to current directory
    PathBuf::from("skills")
}

/// Registry of installed skills.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    /// Installed skills keyed by name.
    skills: HashMap<String, InstalledSkill>,
    /// Skills directory.
    skills_dir: PathBuf,
    /// When true, no new skills can be loaded (Stable mode).
    frozen: bool,
}

impl SkillRegistry {
    /// Create a new registry rooted at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dir,
            frozen: false,
        }
    }

    /// Create a cheap owned snapshot of this registry.
    ///
    /// Used to avoid holding `RwLockReadGuard` across `.await` points
    /// (the guard is `!Send`).
    pub fn snapshot(&self) -> SkillRegistry {
        SkillRegistry {
            skills: self.skills.clone(),
            skills_dir: self.skills_dir.clone(),
            frozen: self.frozen,
        }
    }

    /// Freeze the registry, preventing any new skills from being loaded.
    /// Used in Stable mode after initial boot.
    pub fn freeze(&mut self) {
        self.frozen = true;
        info!("Skill registry frozen — no new skills will be loaded");
    }

    /// Check if the registry is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Load all installed skills from the skills directory.
    pub fn load_all(&mut self) -> Result<usize, SkillError> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&self.skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_skill(&path) {
                Ok(_) => count += 1,
                Err(e) => {
                    warn!("Failed to load skill at {}: {e}", path.display());
                }
            }
        }

        info!("Loaded {count} skills from {}", self.skills_dir.display());
        Ok(count)
    }

    /// Load a single skill from a directory.
    pub fn load_skill(&mut self, skill_dir: &Path) -> Result<String, SkillError> {
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }
        let manifest_path = skill_dir.join("skill.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: SkillManifest = toml::from_str(&toml_str)?;

        let name = manifest.skill.name.clone();

        self.skills.insert(
            name.clone(),
            InstalledSkill {
                manifest,
                path: skill_dir.to_path_buf(),
                enabled: true,
            },
        );

        info!("Loaded skill: {name}");
        Ok(name)
    }

    /// Get an installed skill by name.
    pub fn get(&self, name: &str) -> Option<&InstalledSkill> {
        self.skills.get(name)
    }

    /// List all installed skills.
    pub fn list(&self) -> Vec<&InstalledSkill> {
        self.skills.values().collect()
    }

    /// Remove a skill by name.
    pub fn remove(&mut self, name: &str) -> Result<(), SkillError> {
        let skill = self
            .skills
            .remove(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

        // Remove the skill directory
        if skill.path.exists() {
            std::fs::remove_dir_all(&skill.path)?;
        }

        info!("Removed skill: {name}");
        Ok(())
    }

    /// Get all tool definitions from all enabled skills.
    pub fn all_tool_definitions(&self) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled)
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Get tool definitions only from the named skills.
    pub fn tool_definitions_for_skills(&self, names: &[String]) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled && names.contains(&s.manifest.skill.name))
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Return all installed skill names.
    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    /// Find which skill provides a given tool name.
    pub fn find_tool_provider(&self, tool_name: &str) -> Option<&InstalledSkill> {
        self.skills.values().find(|s| {
            s.enabled
                && s.manifest
                    .tools
                    .provided
                    .iter()
                    .any(|t| t.name == tool_name)
        })
    }

    /// Count installed skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Load workspace-scoped skills that override global/bundled skills.
    ///
    /// Scans subdirectories of `workspace_skills_dir` using the same loading
    /// logic as `load_all()`: auto-converts SKILL.md, runs prompt injection
    /// scan, blocks critical threats. Skills loaded here override global ones
    /// with the same name (insert semantics).
    pub fn load_workspace_skills(
        &mut self,
        workspace_skills_dir: &Path,
    ) -> Result<usize, SkillError> {
        if !workspace_skills_dir.exists() {
            return Ok(0);
        }
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }

        let mut count = 0;
        let entries = std::fs::read_dir(workspace_skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_skill(&path) {
                Ok(name) => {
                    info!("Loaded workspace skill: {name}");
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to load workspace skill at {}: {e}", path.display());
                }
            }
        }

        if count > 0 {
            info!(
                "Loaded {count} workspace skill(s) from {}",
                workspace_skills_dir.display()
            );
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"
[skill]
name = "{name}"
version = "0.1.0"
description = "Test skill"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_load_all() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "skill-a");
        create_test_skill(dir.path(), "skill-b");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 2);
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_get_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "my-skill");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let skill = registry.get("my-skill");
        assert!(skill.is_some());
        assert_eq!(skill.unwrap().manifest.skill.name, "my-skill");
    }

    #[test]
    fn test_tool_definitions() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "alpha");
        create_test_skill(dir.path(), "beta");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let tools = registry.all_tool_definitions();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_find_tool_provider() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "finder");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        assert!(registry.find_tool_provider("finder_tool").is_some());
        assert!(registry.find_tool_provider("nonexistent").is_none());
    }

    #[test]
    fn test_remove_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "removable");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 1);

        registry.remove("removable").unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_empty_dir() {
        let dir = TempDir::new().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        assert_eq!(registry.load_all().unwrap(), 0);
    }

    #[test]
    fn test_frozen_blocks_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "blocked");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.freeze();
        assert!(registry.is_frozen());

        // Trying to load a skill should fail
        let result = registry.load_skill(&dir.path().join("blocked"));
        assert!(result.is_err());
    }

    #[test]
    fn test_frozen_after_initial_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "initial");
        create_test_skill(dir.path(), "later");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        // Initial load works
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 2);

        // Freeze
        registry.freeze();

        // Dynamic load blocked
        create_test_skill(dir.path(), "new-skill");
        let result = registry.load_skill(&dir.path().join("new-skill"));
        assert!(result.is_err());
        // Still has the original skills
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_resolve_skills_dir_env_priority() {
        // Test environment variable priority in a single test to avoid race conditions

        // Clear all env vars first
        std::env::remove_var("YINGHE_SKILLS_DIR");
        std::env::remove_var("OPENCARRIER_SKILLS_DIR");

        // Test YINGHE_SKILLS_DIR has highest priority
        let temp_dir1 = TempDir::new().unwrap();
        let yinghe_path = temp_dir1.path().join("yinghe_skills");
        std::fs::create_dir_all(&yinghe_path).unwrap();

        let temp_dir2 = TempDir::new().unwrap();
        let oc_path = temp_dir2.path().join("oc_skills");
        std::fs::create_dir_all(&oc_path).unwrap();

        // Set both env vars - YINGHE should win
        std::env::set_var("YINGHE_SKILLS_DIR", &yinghe_path);
        std::env::set_var("OPENCARRIER_SKILLS_DIR", &oc_path);
        let resolved = resolve_skills_dir();
        std::env::remove_var("YINGHE_SKILLS_DIR");

        assert_eq!(
            resolved, yinghe_path,
            "YINGHE_SKILLS_DIR should have priority"
        );

        // Now test OPENCARRIER_SKILLS_DIR when YINGHE is not set
        let resolved = resolve_skills_dir();
        std::env::remove_var("OPENCARRIER_SKILLS_DIR");

        assert_eq!(
            resolved, oc_path,
            "OPENCARRIER_SKILLS_DIR should be used as fallback"
        );
    }
}
