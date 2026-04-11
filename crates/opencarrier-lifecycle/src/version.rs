//! Knowledge version management — change tracking via JSONL log.
//!
//! Ported from openclone-core/src/version.rs with path adaptation:
//! - `clone_dir/history/versions.jsonl` → `workspace/history/versions.jsonl`
//! - Uses `chrono` for timestamps instead of raw Unix epoch

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

/// A single version record.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VersionEntry {
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Action type: create / update / delete / verify / rollback
    pub action: String,
    /// Filename (relative to data/knowledge/)
    pub file: String,
    /// Content before change (None = new file)
    pub before: Option<String>,
    /// Content after change (None = deleted)
    pub after: Option<String>,
    /// Source: evolution / user / verify / rollback
    pub source: String,
    /// Whether a human has verified this change
    pub verified: bool,
}

/// Append a version record to the JSONL log.
pub fn record_version(
    workspace: &Path,
    action: &str,
    file: &str,
    before: Option<&str>,
    after: Option<&str>,
    source: &str,
) -> Result<()> {
    let history_dir = workspace.join("history");
    fs::create_dir_all(&history_dir)?;

    let versions_path = history_dir.join("versions.jsonl");
    let entry = VersionEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        action: action.to_string(),
        file: file.to_string(),
        before: before.map(String::from),
        after: after.map(String::from),
        source: source.to_string(),
        verified: source == "user" || source == "verify",
    };

    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&versions_path)?;
    f.write_all(line.as_bytes())?;

    Ok(())
}

/// Get version history for a specific file.
pub fn get_file_history(workspace: &Path, filename: &str) -> Result<Vec<VersionEntry>> {
    let versions_path = workspace.join("history/versions.jsonl");
    if !versions_path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&versions_path)?;
    let entries: Vec<VersionEntry> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<VersionEntry>(line).ok())
        .filter(|e| e.file == filename)
        .collect();

    Ok(entries)
}

/// Get all version records.
pub fn get_all_versions(workspace: &Path) -> Result<Vec<VersionEntry>> {
    let versions_path = workspace.join("history/versions.jsonl");
    if !versions_path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&versions_path)?;
    let entries: Vec<VersionEntry> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<VersionEntry>(line).ok())
        .collect();

    Ok(entries)
}

/// List unverified knowledge entries (pending human review).
pub fn list_unverified(workspace: &Path) -> Result<Vec<VersionEntry>> {
    let versions = get_all_versions(workspace)?;
    Ok(versions
        .into_iter()
        .filter(|v| !v.verified && v.action != "delete" && v.action != "rollback")
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_record_and_read_version() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        record_version(workspace, "create", "test-knowledge.md", None, Some("content"), "evolution")
            .unwrap();

        let history = get_file_history(workspace, "test-knowledge.md").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].action, "create");
        assert_eq!(history[0].file, "test-knowledge.md");
        assert!(history[0].before.is_none());
        assert_eq!(history[0].after.as_deref(), Some("content"));
        assert!(!history[0].verified);
    }

    #[test]
    fn test_multiple_versions() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        record_version(workspace, "create", "a.md", None, Some("v1"), "evolution").unwrap();
        record_version(workspace, "update", "a.md", Some("v1"), Some("v2"), "evolution").unwrap();
        record_version(workspace, "create", "b.md", None, Some("other"), "user").unwrap();

        let all = get_all_versions(workspace).unwrap();
        assert_eq!(all.len(), 3);

        let a_history = get_file_history(workspace, "a.md").unwrap();
        assert_eq!(a_history.len(), 2);

        let unverified = list_unverified(workspace).unwrap();
        assert_eq!(unverified.len(), 2); // a.md create + update (evolution-sourced)
    }

    #[test]
    fn test_user_source_is_verified() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        record_version(workspace, "create", "x.md", None, Some("content"), "user").unwrap();

        let history = get_file_history(workspace, "x.md").unwrap();
        assert!(history[0].verified);
    }

    #[test]
    fn test_empty_history() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        let history = get_file_history(workspace, "nonexistent.md").unwrap();
        assert!(history.is_empty());

        let all = get_all_versions(workspace).unwrap();
        assert!(all.is_empty());
    }
}
