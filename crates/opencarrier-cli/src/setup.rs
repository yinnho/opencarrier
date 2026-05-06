//! First-run setup: device-based auto-registration with Hub.
//!
//! No user interaction needed. Generates random device_id + credentials,
//! registers with Hub, obtains API key, writes config, enables auto-login.
//! User can change username/password later in the dashboard UI.

use colored::Colorize;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct AuthResponse {
    token: String,
}

#[derive(Deserialize)]
struct KeyResponse {
    key: String,
}

/// Generate a random alphanumeric string of the given length.
fn random_string(len: usize) -> String {
    use rand::Rng;
    let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| charset[rng.gen_range(0..charset.len())] as char)
        .collect()
}

/// Generate a random password (alphanumeric, mixed case + digits).
fn random_password(len: usize) -> String {
    use rand::Rng;
    let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| charset[rng.gen_range(0..charset.len())] as char)
        .collect()
}

/// Run the first-run setup flow. Zero interaction — device is identity.
/// Returns (username, password) if config was written.
pub fn run_first_time_setup(
    opencarrier_dir: &Path,
    hub_url: &str,
) -> Result<(String, String), String> {
    println!();
    println!(
        "  {} {}",
        ">>".bright_cyan().bold(),
        "Setting up OpenCarrier".bold()
    );
    println!("  {}", "Registering device with Hub...".dimmed());
    println!();

    // Generate device_id → saved to ~/.opencarrier/device_id
    let device_id = opencarrier_clone::hub::get_or_create_device_id(opencarrier_dir)
        .unwrap_or_else(|_| random_string(32));

    // Auto-generate credentials based on device_id
    let device_short = &device_id[..8.min(device_id.len())];
    let username = format!("dev_{}", device_short);
    let password = random_password(16);
    let email = format!("{}@device.opencarrier", username);

    println!("  {} Registering with {}...", "-".bright_yellow(), hub_url);

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let api_key: String = rt.block_on(async {
        register_and_get_key(hub_url, &username, &email, &password, &device_id).await
    })?;

    // Save API key to .env
    let env_path = opencarrier_dir.join(".env");
    let env_content = format!("OPENCLONE_HUB_KEY={}\n", api_key);
    std::fs::write(&env_path, &env_content).map_err(|e| e.to_string())?;
    crate::restrict_file_permissions(&env_path);

    // Write config.toml with auth enabled
    let password_hash = opencarrier_api::session_auth::hash_password(&password);
    let config_content = format!(
        r#"# OpenCarrier Agent OS configuration
api_listen = "127.0.0.1:4200"

[brain]
config = "brain.json"

[memory]
decay_rate = 0.05

[auth]
enabled = true
username = "{username}"
password_hash = "{password_hash}"
session_ttl_hours = 168
"#
    );
    let config_path = opencarrier_dir.join("config.toml");
    std::fs::write(&config_path, &config_content).map_err(|e| e.to_string())?;
    crate::restrict_file_permissions(&config_path);

    // Load .env into current process so the kernel picks it up.
    // Called before tokio runtime starts, so no concurrent env access.
    std::env::set_var("OPENCLONE_HUB_KEY", &api_key);

    println!(
        "  {} Device registered and API key saved!",
        "\u{2714}".bright_green()
    );
    println!("  {} Username: {}", "\u{2714}".bright_green(), username);
    println!();

    Ok((username, password))
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

/// Save the plain password for auto-login (stored in restricted file).
pub fn save_login_secret(opencarrier_dir: &Path, password: &str) -> Result<(), String> {
    let secret_path = opencarrier_dir.join(".login");
    std::fs::write(&secret_path, password).map_err(|e| e.to_string())?;
    crate::restrict_file_permissions(&secret_path);
    Ok(())
}

/// Read the saved login password.
pub fn read_login_secret(opencarrier_dir: &Path) -> Option<String> {
    let secret_path = opencarrier_dir.join(".login");
    let password = std::fs::read_to_string(secret_path).ok()?;
    let p = password.trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

async fn register_and_get_key(
    hub_url: &str,
    username: &str,
    email: &str,
    password: &str,
    device_id: &str,
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
        if status == reqwest::StatusCode::CONFLICT {
            println!(
                "  {} Device already registered, logging in...",
                "-".bright_yellow()
            );
            return login_and_get_key(hub_url, username, password, device_id).await;
        }
        return Err(format!("Registration failed ({}): {}", status, body));
    }

    let auth: AuthResponse = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    // Create API key (bound to this device)
    let key_resp = client
        .post(format!("{}/api/keys", base))
        .bearer_auth(&auth.token)
        .header("X-Device-ID", device_id)
        .json(&serde_json::json!({ "name": "opencarrier" }))
        .send()
        .await
        .map_err(|e| format!("Key creation failed: {e}"))?;

    if !key_resp.status().is_success() {
        let body = key_resp.text().await.unwrap_or_default();
        return Err(format!("Failed to create API key: {}", body));
    }

    let key_data: KeyResponse = key_resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;
    Ok(key_data.key)
}

async fn login_and_get_key(
    hub_url: &str,
    username: &str,
    password: &str,
    device_id: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let base = hub_url.trim_end_matches('/');

    let resp = client
        .post(format!("{}/api/auth/login", base))
        .json(&serde_json::json!({
            "login": username,
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

    // Create new API key (bound to this device)
    let key_resp = client
        .post(format!("{}/api/keys", base))
        .bearer_auth(&auth.token)
        .header("X-Device-ID", device_id)
        .json(&serde_json::json!({ "name": "opencarrier" }))
        .send()
        .await
        .map_err(|e| format!("Key creation failed: {e}"))?;

    if !key_resp.status().is_success() {
        let body = key_resp.text().await.unwrap_or_default();
        return Err(format!("Failed to create API key: {}", body));
    }

    let key_data: KeyResponse = key_resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;
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
