//! Workspace filesystem sandboxing.
//!
//! Confines agent file operations to their workspace directory.
//! Prevents path traversal, symlink escapes, and access outside the sandbox.

use std::path::{Path, PathBuf};

/// Resolve a user-supplied path within a workspace sandbox.
///
/// - Rejects `..` components outright.
/// - Relative paths are joined with `workspace_root`.
/// - Absolute paths are checked against the workspace root after canonicalization.
/// - For new files: canonicalizes the parent directory and appends the filename.
/// - The final canonical path must start with the canonical workspace root.
pub fn resolve_sandbox_path(user_path: &str, workspace_root: &Path) -> Result<PathBuf, String> {
    let path = Path::new(user_path);

    // Reject any `..` components
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }

    // Build the candidate path
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };

    // Canonicalize the workspace root
    let canon_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve workspace root: {e}"))?;

    // Canonicalize the candidate (or its parent for new files)
    let canon_candidate = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"))?
    } else {
        // For new files: find the nearest existing ancestor, canonicalize it,
        // then re-append the remaining path components and create intermediate dirs.
        // Collect path components from leaf to ancestor (e.g. ["file.md", "subdir", "knowledge"])
        let mut ancestor = candidate.clone();
        let mut components: Vec<std::ffi::OsString> = Vec::new();

        loop {
            let name = ancestor
                .file_name()
                .ok_or_else(|| "Invalid path: no filename".to_string())?
                .to_os_string();
            components.push(name);

            let parent = ancestor
                .parent()
                .ok_or_else(|| "Invalid path: no parent directory".to_string())?;

            if parent.exists() {
                let canon_parent = parent
                    .canonicalize()
                    .map_err(|e| format!("Failed to resolve parent directory: {e}"))?;
                // Verify the existing ancestor is inside the sandbox
                if !canon_parent.starts_with(&canon_root) {
                    return Err(format!(
                        "Access denied: path '{}' resolves outside workspace",
                        user_path
                    ));
                }

                // components was collected leaf-to-ancestor, rev gives ancestor-to-leaf
                // e.g. ["knowledge", "subdir", "file.md"]
                // Create directories for all but the last component (the filename)
                let rev: Vec<_> = components.into_iter().rev().collect();
                let mut current = canon_parent.clone();
                for part in rev.iter().take(rev.len() - 1) {
                    current = current.join(part);
                    if !current.exists() {
                        std::fs::create_dir(&current).map_err(|e| {
                            format!("Failed to create directory '{}': {e}", current.display())
                        })?;
                    }
                }
                // Append the filename (last component)
                break current.join(rev.last().unwrap());
            }
            ancestor = parent.to_path_buf();
        }
    };

    // Verify the canonical path is inside the workspace
    if !canon_candidate.starts_with(&canon_root) {
        return Err(format!(
            "Access denied: path '{}' resolves outside workspace. \
             If you have an MCP filesystem server configured, use the \
             mcp_filesystem_* tools (e.g. mcp_filesystem_read_file, \
             mcp_filesystem_list_directory) to access files outside \
             the workspace.",
            user_path
        ));
    }

    Ok(canon_candidate)
}

/// Resolve a user-supplied path for write operations within a workspace sandbox.
///
/// Extends `resolve_sandbox_path` with per-directory permission rules:
/// - **Blocked**: `agent.toml`, `SOUL.md` (only trainer tools may modify these)
/// - **Allowed (self-evolution)**: `system_prompt.md`, `skills/`, `data/`, `memory/`, `output/`
/// - **Per-user**: `users/{sender_id}/` when sender_id matches the current sender
/// - **Blocked**: `users/{other_sender_id}/`
///
/// When `sender_id` is present and the path starts with `output/`, the path is
/// automatically rewritten to `users/{sender_id}/output/`.
pub fn resolve_sandbox_path_for_write(
    user_path: &str,
    workspace_root: &Path,
    sender_id: Option<&str>,
) -> Result<PathBuf, String> {
    let normalized = user_path.replace('\\', "/");
    let path = Path::new(&normalized);

    // Extract the relative path components for permission checking
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace_root)
            .map_err(|_| "Absolute path outside workspace".to_string())?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };

    let rel_str = relative.to_string_lossy();

    // Block writes to protected config files
    if rel_str == "agent.toml" || rel_str == "SOUL.md" {
        return Err(format!(
            "Write denied: '{}' is a protected config file (only trainer may modify)",
            rel_str
        ));
    }

    // Rewrite output/ paths to per-user output when sender_id is present
    let effective_path = if let Some(sid) = sender_id {
        if rel_str.starts_with("output/") || rel_str == "output" {
            let rest = rel_str.strip_prefix("output").unwrap_or("");
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            if rest.is_empty() {
                format!("users/{}/output", sid)
            } else {
                format!("users/{}/output/{}", sid, rest)
            }
        } else {
            rel_str.to_string()
        }
    } else {
        rel_str.to_string()
    };

    // Check per-user isolation for users/ paths
    let eff_path = Path::new(&effective_path);
    if eff_path.starts_with("users/") {
        let components: Vec<&str> = eff_path.components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        // components: ["users", "{sender_id}", ...]
        if components.len() >= 2 {
            let path_sender = components[1];
            if let Some(sid) = sender_id {
                if path_sender != sid {
                    return Err(format!(
                        "Write denied: cannot write to user '{}' directory (current sender: '{}')",
                        path_sender, sid
                    ));
                }
            } else {
                return Err("Write denied: cannot write to users/ directory without sender context".to_string());
            }
        }
    }

    // Delegate to the existing sandbox for path resolution and traversal checks
    resolve_sandbox_path(&effective_path, workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_relative_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("test.txt"), "hello").unwrap();

        let result = resolve_sandbox_path("data/test.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_absolute_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "ok").unwrap();
        let abs_path = dir.path().join("file.txt");

        let result = resolve_sandbox_path(abs_path.to_str().unwrap(), dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_absolute_path_outside_workspace_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = std::env::temp_dir().join("outside_test.txt");
        std::fs::write(&outside, "nope").unwrap();

        let result = resolve_sandbox_path(outside.to_str().unwrap(), dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn test_dotdot_component_blocked() {
        let dir = TempDir::new().unwrap();
        let result = resolve_sandbox_path("../../../etc/passwd", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal denied"));
    }

    #[test]
    fn test_nonexistent_file_with_valid_parent() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let result = resolve_sandbox_path("data/new_file.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[test]
    fn test_nonexistent_file_with_missing_parent_dirs() {
        let dir = TempDir::new().unwrap();

        // knowledge/ does NOT exist yet — this is the failing case
        let result = resolve_sandbox_path("knowledge/city-beijing.md", dir.path());
        assert!(result.is_ok(), "Expected OK, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("city-beijing.md"));
        // The intermediate directory should have been created
        assert!(resolved.parent().unwrap().exists());
    }

    #[test]
    fn test_nonexistent_file_with_deeply_missing_parents() {
        let dir = TempDir::new().unwrap();

        // Neither skills/ nor sub/ exists
        let result = resolve_sandbox_path("skills/sub/deep/file.md", dir.path());
        assert!(result.is_ok(), "Expected OK, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("file.md"));
        assert!(resolved.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        // Create a symlink inside the workspace pointing outside
        let link_path = dir.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link_path).unwrap();

        let result = resolve_sandbox_path("escape/secret.txt", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));
    }
}
