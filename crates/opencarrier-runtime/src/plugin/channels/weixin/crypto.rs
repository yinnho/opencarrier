//! AES-128-ECB encryption/decryption for WeChat iLink CDN media.
//!
//! All CDN media uses AES-128-ECB with PKCS7-style padding.
//! Ciphertext size = ceil((plaintext_size + 1) / 16) * 16.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use base64::Engine;
use reqwest::Client;
use tracing::warn;

use crate::plugin::channels::weixin::types::CDN_BASE_URL;

type Aes128 = aes::Aes128;

/// Compute AES-ECB padded ciphertext size.
pub fn aes_ecb_padded_size(plaintext_len: usize) -> usize {
    (plaintext_len + 1).div_ceil(16) * 16
}

/// AES-128-ECB encrypt with iLink padding.
pub fn aes_128_ecb_encrypt(plaintext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let padded_len = aes_ecb_padded_size(plaintext.len());
    let mut buf = vec![0u8; padded_len];
    buf[..plaintext.len()].copy_from_slice(plaintext);
    // iLink pads with zeros (the +1 in padded_size handles the terminator)

    for chunk in buf.chunks_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    buf
}

/// AES-128-ECB decrypt. Returns plaintext (trailing zeros trimmed).
pub fn aes_128_ecb_decrypt(ciphertext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut buf = ciphertext.to_vec();

    for chunk in buf.chunks_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }

    // Trim trailing zeros
    let end = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
    buf.truncate(end);
    buf
}

/// Parse AES key from CDNMedia.aes_key field.
///
/// The key can be:
/// - Raw 16 bytes: base64_decode(aes_key) = 16 raw bytes
/// - Hex-encoded: base64_decode(aes_key) = 32 ASCII hex chars → 16 bytes
pub fn parse_aes_key(aes_key_b64: &str) -> Option<[u8; 16]> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(aes_key_b64)
        .ok()?;

    match decoded.len() {
        16 => {
            let mut key = [0u8; 16];
            key.copy_from_slice(&decoded);
            Some(key)
        }
        32 => {
            // Check if it's hex-encoded
            let s = std::str::from_utf8(&decoded).ok()?;
            if s.chars().all(|c| c.is_ascii_hexdigit()) {
                hex::decode_to_slice(s, &mut [0u8; 16]).ok()?;
                let mut key = [0u8; 16];
                hex::decode_to_slice(s, &mut key).ok()?;
                Some(key)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Generate a random 16-byte AES key and return (raw_bytes, hex_string).
pub fn generate_aes_key() -> ([u8; 16], String) {
    use rand::Rng;
    let mut key = [0u8; 16];
    rand::thread_rng().fill(&mut key);
    let hex_str = hex::encode(key);
    (key, hex_str)
}

/// Download and decrypt a file from CDN.
pub async fn cdn_download(
    http: &Client,
    url: &str,
    key: &[u8; 16],
) -> Result<Vec<u8>, String> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("CDN download failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CDN download HTTP {status}: {body}"));
    }

    let ciphertext = resp
        .bytes()
        .await
        .map_err(|e| format!("CDN download read error: {e}"))?;

    Ok(aes_128_ecb_decrypt(&ciphertext, key))
}

/// Upload encrypted file to CDN. Returns the download encrypted_query_param.
pub async fn cdn_upload(
    http: &Client,
    upload_url: &str,
    ciphertext: &[u8],
) -> Result<String, String> {
    let resp = http
        .post(upload_url)
        .header("Content-Type", "application/octet-stream")
        .body(ciphertext.to_vec())
        .send()
        .await
        .map_err(|e| format!("CDN upload failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CDN upload HTTP {status}: {body}"));
    }

    // Extract download param from response header
    let download_param = resp
        .headers()
        .get("x-encrypted-param")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(ref err) = resp.headers().get("x-error-message") {
        warn!(error = ?err, "CDN upload warning");
    }

    download_param.ok_or_else(|| "CDN upload: no x-encrypted-param in response".to_string())
}

/// Build CDN download URL from encrypt_query_param.
pub fn cdn_download_url(encrypt_query_param: &str) -> String {
    format!(
        "{}/download?encrypted_query_param={}",
        CDN_BASE_URL,
        urlencoding::encode(encrypt_query_param)
    )
}

/// Compute MD5 hex digest of data.
pub fn md5_hex(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
