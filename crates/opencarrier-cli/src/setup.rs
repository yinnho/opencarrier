//! First-run setup: register with Hub, obtain API key, write config.

use colored::Colorize;
use serde::Deserialize;
use std::io::{self, BufRead, Write};
use std::path::Path;

#[derive(Deserialize)]
struct AuthResponse {
    token: String,
}

#[derive(Deserialize)]
struct KeyResponse {
    key: String,
}

/// Run the first-run setup flow. Returns Ok(()) if config was written.
pub fn run_first_time_setup(opencarrier_dir: &Path, hub_url: &str) -> Result<(), String> {
    println!();
    println!("  {} {}", ">>".bright_cyan().bold(), "First-time Setup".bold());
    println!("  {}", "This will create a Hub account and configure OpenCarrier.".dimmed());
    println!();

    let stdin = io::stdin();
    let mut input = String::new();

    // Username
    print!("  {}: ", "Username".bold());
    io::stdout().flush().map_err(|e| e.to_string())?;
    input.clear();
    stdin.lock().read_line(&mut input).map_err(|e| e.to_string())?;
    let username = input.trim().to_string();
    if username.is_empty() {
        return Err("Username cannot be empty".to_string());
    }

    // Email
    print!("  {}: ", "Email".bold());
    io::stdout().flush().map_err(|e| e.to_string())?;
    input.clear();
    stdin.lock().read_line(&mut input).map_err(|e| e.to_string())?;
    let email = input.trim().to_string();
    if email.is_empty() {
        return Err("Email cannot be empty".to_string());
    }

    // Password
    print!("  {}: ", "Password".bold());
    io::stdout().flush().map_err(|e| e.to_string())?;
    input.clear();
    stdin.lock().read_line(&mut input).map_err(|e| e.to_string())?;
    let password = input.trim().to_string();
    if password.len() < 6 {
        return Err("Password must be at least 6 characters".to_string());
    }

    println!();
    println!("  {} Registering with {}...", "-".bright_yellow(), hub_url);

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let api_key: String = rt.block_on(async {
        register_and_get_key(hub_url, &username, &email, &password).await
    })?;

    // Save API key to .env
    let env_path = opencarrier_dir.join(".env");
    let env_content = format!("OPENCLONE_HUB_KEY={}\n", api_key);
    std::fs::write(&env_path, &env_content).map_err(|e| e.to_string())?;
    crate::restrict_file_permissions(&env_path);

    // Write default config.toml
    let config_path = opencarrier_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, crate::DEFAULT_CONFIG_TOML).map_err(|e| e.to_string())?;
        crate::restrict_file_permissions(&config_path);
    }

    // Load .env into current process so the kernel picks it up
    std::env::set_var("OPENCLONE_HUB_KEY", &api_key);

    println!("  {} Account created and API key saved!", "\u{2714}".bright_green());
    let masked = if api_key.len() > 6 {
        format!("{}***", &api_key[..6])
    } else {
        "***".to_string()
    };
    println!("  {} API key: {}", "\u{2714}".bright_green(), masked);
    println!();

    Ok(())
}

/// Check if first-run setup is needed (no config.toml or no Hub API key).
pub fn needs_setup(opencarrier_dir: &Path) -> bool {
    let config_path = opencarrier_dir.join("config.toml");
    if !config_path.exists() {
        return true;
    }
    // Check if .env exists with OPENCLONE_HUB_KEY
    let env_path = opencarrier_dir.join(".env");
    if let Ok(content) = std::fs::read_to_string(&env_path) {
        for line in content.lines() {
            if let Some(key) = line.strip_prefix("OPENCLONE_HUB_KEY=") {
                if !key.trim().is_empty() {
                    return false;
                }
            }
        }
    }
    // Also check env var
    if let Ok(v) = std::env::var("OPENCLONE_HUB_KEY") {
        if !v.trim().is_empty() {
            return false;
        }
    }
    true
}

async fn register_and_get_key(
    hub_url: &str,
    username: &str,
    email: &str,
    password: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let base = hub_url.trim_end_matches('/');

    // Register
    let resp = client
        .post(format!("{}/api/auth/register", base))
        .json(&serde_json::json!({
            "username": username,
            "email": email,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        // 409 CONFLICT = username/email already taken → try login
        if status == reqwest::StatusCode::CONFLICT {
            println!("  {} Account exists, logging in...", "-".bright_yellow());
            return login_and_get_key(hub_url, username, password).await;
        }
        return Err(format!("Registration failed ({}): {}", status, body));
    }

    let auth: AuthResponse = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    // Create API key
    let key_resp = client
        .post(format!("{}/api/keys", base))
        .bearer_auth(&auth.token)
        .json(&serde_json::json!({ "name": "opencarrier" }))
        .send()
        .await
        .map_err(|e| format!("Key creation failed: {e}"))?;

    if !key_resp.status().is_success() {
        let body = key_resp.text().await.unwrap_or_default();
        return Err(format!("Failed to create API key: {}", body));
    }

    let key_data: KeyResponse = key_resp.json().await.map_err(|e| format!("Parse error: {e}"))?;
    Ok(key_data.key)
}

async fn login_and_get_key(
    hub_url: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let base = hub_url.trim_end_matches('/');

    let resp = client
        .post(format!("{}/api/auth/login", base))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Login failed: {}", body));
    }

    let auth: AuthResponse = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    // Check if there's already a key named "opencarrier"
    let list_resp = client
        .get(format!("{}/api/keys", base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .map_err(|e| format!("Key list failed: {e}"))?;

    if list_resp.status().is_success() {
        let keys: serde_json::Value = list_resp.json().await.unwrap_or_default();
        if let Some(keys_arr) = keys.as_array() {
            for k in keys_arr {
                if k["name"].as_str() == Some("opencarrier") {
                    if let Some(key) = k["key"].as_str() {
                        return Ok(key.to_string());
                    }
                }
            }
        }
    }

    // Create new API key
    let key_resp = client
        .post(format!("{}/api/keys", base))
        .bearer_auth(&auth.token)
        .json(&serde_json::json!({ "name": "opencarrier" }))
        .send()
        .await
        .map_err(|e| format!("Key creation failed: {e}"))?;

    if !key_resp.status().is_success() {
        let body = key_resp.text().await.unwrap_or_default();
        return Err(format!("Failed to create API key: {}", body));
    }

    let key_data: KeyResponse = key_resp.json().await.map_err(|e| format!("Parse error: {e}"))?;
    Ok(key_data.key)
}

/// Check for updates on Hub. Returns the latest version string if newer than current.
pub async fn check_for_update(hub_url: &str) -> Option<String> {
    let base = hub_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/releases", base))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let latest = data["latest"].as_str()?;

    let current = env!("CARGO_PKG_VERSION");
    if latest != current && !latest.is_empty() {
        Some(latest.to_string())
    } else {
        None
    }
}
