//! WeCom crypto — AES decryption, SHA1 signature verification, XML parsing.
//!
//! Migrated from openfang-channels/src/wecom.rs.

use sha1::{Digest, Sha1};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// AES decryption
// ---------------------------------------------------------------------------

/// Decrypt AES-256-CBC with custom PKCS7 padding used by WeCom.
pub fn decrypt_aes_cbc(key: &[u8], encrypted_base64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    use cbc::cipher::{BlockDecryptMut, KeyIvInit};

    let mut encrypted = base64::engine::general_purpose::STANDARD
        .decode(encrypted_base64)
        .map_err(|e| format!("base64 decode error: {e}"))?;

    type Aes256CbcDecrypt = cbc::Decryptor<aes::Aes256>;
    let iv = &key[..16];
    let cipher = Aes256CbcDecrypt::new(key.into(), iv.into());

    let decrypted = cipher
        .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut encrypted)
        .map_err(|e| format!("decrypt error: {e}"))?;

    let decrypted = decrypted.to_vec();
    let pad = decrypted
        .last()
        .copied()
        .ok_or_else(|| "decrypted payload is empty".to_string())? as usize;

    if pad == 0 || pad > 32 || decrypted.len() < pad {
        return Err(format!("invalid WeCom PKCS7 padding length: {pad}"));
    }
    if !decrypted[decrypted.len() - pad..]
        .iter()
        .all(|byte| *byte as usize == pad)
    {
        return Err("invalid WeCom PKCS7 padding bytes".to_string());
    }

    Ok(decrypted[..decrypted.len() - pad].to_vec())
}

// ---------------------------------------------------------------------------
// Signature verification
// ---------------------------------------------------------------------------

/// Verify WeCom callback signature (SHA1 of sorted [token, timestamp, nonce, encrypted_payload]).
pub fn is_valid_wecom_signature(
    token: &str,
    timestamp: &str,
    nonce: &str,
    encrypted_payload: &str,
    msg_signature: &str,
) -> bool {
    let mut parts = [token, timestamp, nonce, encrypted_payload];
    parts.sort_unstable();

    let mut hasher = Sha1::new();
    hasher.update(parts.concat().as_bytes());
    hex::encode(hasher.finalize()) == msg_signature
}

// ---------------------------------------------------------------------------
// Payload decoding
// ---------------------------------------------------------------------------

/// Decode a WeCom encrypted payload using the encoding AES key.
///
/// Returns the decrypted message text.
pub fn decode_wecom_payload(encoding_aes_key: &str, encrypted_payload: &str) -> Result<String, String> {
    use base64::{
        alphabet,
        engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    let aes_key_engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireNone)
            .with_decode_allow_trailing_bits(true),
    );

    let aes_key = aes_key_engine
        .decode(encoding_aes_key)
        .map_err(|e| format!("aes key decode error: {e}"))?;
    let decrypted = decrypt_aes_cbc(&aes_key, encrypted_payload)?;

    if decrypted.len() < 20 {
        return Err("decrypted payload too short".to_string());
    }

    let msg_len =
        u32::from_be_bytes([decrypted[16], decrypted[17], decrypted[18], decrypted[19]]) as usize;
    if decrypted.len() < 20 + msg_len {
        return Err("decrypted payload shorter than declared message".to_string());
    }

    String::from_utf8(decrypted[20..20 + msg_len].to_vec())
        .map_err(|e| format!("payload is not valid utf-8: {e}"))
}

// ---------------------------------------------------------------------------
// XML parsing
// ---------------------------------------------------------------------------

/// Parse WeCom callback XML into a flat HashMap.
pub fn parse_wecom_xml_fields(xml: &str) -> Result<HashMap<String, String>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("invalid xml: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "xml" {
        return Err("root element is not <xml>".to_string());
    }

    let mut fields = HashMap::new();
    for child in root.children().filter(|node| node.is_element()) {
        let value = child
            .children()
            .filter_map(|node| node.text())
            .collect::<String>()
            .trim()
            .to_string();
        fields.insert(child.tag_name().name().to_string(), value);
    }

    Ok(fields)
}
