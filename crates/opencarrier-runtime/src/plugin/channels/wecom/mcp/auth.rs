//! MCP authentication — SHA256 signature for get_mcp_config requests.

use sha2::{Digest, Sha256};

/// Generate the MCP config request signature.
///
/// Algorithm: `SHA256_hex(secret + bot_id + time_string + nonce)`
pub fn generate_signature(bot_id: &str, secret: &str, time: u64, nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(bot_id.as_bytes());
    hasher.update(time.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Build the request body for `get_mcp_config`.
pub fn build_config_request(bot_id: &str, secret: &str) -> serde_json::Value {
    use rand::Rng;
    use std::time::{SystemTime, UNIX_EPOCH};

    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let random_hex: String = rand::thread_rng()
        .sample_iter(rand::distributions::Alphanumeric)
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect();
    let nonce = format!(
        "mcp_{}_{random_hex}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let signature = generate_signature(bot_id, secret, time, &nonce);

    serde_json::json!({
        "bot_id": bot_id,
        "time": time,
        "nonce": nonce,
        "signature": signature,
        "bind_source": 2,  // Qrcode
        "cli_version": format!("OpenCarrier/{}", env!("CARGO_PKG_VERSION"))
    })
}
