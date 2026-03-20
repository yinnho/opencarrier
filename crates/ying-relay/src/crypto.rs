//! Encryption/Decryption for Relay Messages
//!
//! 使用 ECDH P-256 + AES-256-GCM 进行端到端加密

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use p256::elliptic_curve::ecdh::diffie_hellman;
use p256::PublicKey;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::protocol::EncryptedPayload;

/// 加密数据包
#[derive(Debug, Clone)]
pub struct EncryptedPacket {
    pub version: u8,
    pub timestamp: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

/// ECDH P-256 密钥对
#[derive(Debug, Clone)]
pub struct EcdhKeyPair {
    pub public_key: Vec<u8>,
    pub private_key: Vec<u8>,
}

impl EcdhKeyPair {
    /// 生成新的 ECDH P-256 密钥对
    pub fn generate() -> Self {
        use p256::SecretKey;

        let secret = SecretKey::random(&mut rand::rngs::OsRng);
        // 公钥 = 基点 * 私钥
        let public_key = PublicKey::from_secret_scalar(&secret.to_nonzero_scalar());

        Self {
            public_key: public_key.to_sec1_bytes().to_vec(),
            private_key: secret.to_bytes().to_vec(),
        }
    }
}

/// 计算 ECDH 共享密钥
pub fn compute_shared_secret(
    private_key: &[u8],
    peer_public_key: &[u8],
) -> Vec<u8> {
    use p256::{SecretKey, PublicKey};

    let secret_key = SecretKey::from_slice(private_key).expect("Invalid private key");
    let public_key = PublicKey::from_sec1_bytes(peer_public_key).expect("Invalid public key");

    // ECDH: 使用 diffie_hellman 函数
    let shared_secret = diffie_hellman(
        secret_key.to_nonzero_scalar(),
        public_key.as_affine(),
    );

    // 共享密钥的原始字节
    let shared_bytes = shared_secret.raw_secret_bytes();

    // 派生 32 字节密钥 (AES-256)
    let mut hasher = Sha256::new();
    hasher.update(shared_bytes.as_slice());
    hasher.finalize().to_vec()
}

/// 加密数据
pub fn encrypt(
    plaintext: &str,
    shared_secret: &[u8],
) -> Result<EncryptedPacket, CryptoError> {
    if shared_secret.len() != 32 {
        return Err(CryptoError::InvalidKeyLength);
    }

    let cipher = Aes256Gcm::new_from_slice(shared_secret)
        .map_err(|_| CryptoError::InvalidKey)?;

    // 生成 12 字节随机 nonce
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| CryptoError::EncryptionFailed)?;

    // AES-GCM 的 tag 在 ciphertext 末尾
    let tag_len = 16;
    let (ct, tag) = ciphertext.split_at(ciphertext.len() - tag_len);

    Ok(EncryptedPacket {
        version: 1,
        timestamp: chrono::Utc::now().timestamp_millis(),
        nonce: nonce_bytes.to_vec(),
        ciphertext: ct.to_vec(),
        tag: tag.to_vec(),
    })
}

/// 解密数据
pub fn decrypt(
    packet: &EncryptedPayload,
    shared_secret: &[u8],
) -> Result<String, CryptoError> {
    if shared_secret.len() != 32 {
        return Err(CryptoError::InvalidKeyLength);
    }

    // 验证时间戳 (30秒容差)
    let now = chrono::Utc::now().timestamp_millis();
    if (now - packet.timestamp).abs() > 30000 {
        return Err(CryptoError::TimestampExpired);
    }

    let cipher = Aes256Gcm::new_from_slice(shared_secret)
        .map_err(|_| CryptoError::InvalidKey)?;

    let nonce = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &packet.nonce)
        .map_err(|_| CryptoError::InvalidNonce)?;
    let ciphertext = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &packet.ciphertext)
        .map_err(|_| CryptoError::InvalidCiphertext)?;
    let tag = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &packet.tag)
        .map_err(|_| CryptoError::InvalidTag)?;

    let mut combined = ciphertext.to_vec();
    combined.extend_from_slice(&tag);

    let nonce = Nonce::from_slice(&nonce);
    let plaintext = cipher
        .decrypt(nonce, combined.as_ref())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    String::from_utf8(plaintext).map_err(|_| CryptoError::InvalidUtf8)
}

/// 加密错误
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Invalid key length")]
    InvalidKeyLength,
    #[error("Invalid key")]
    InvalidKey,
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed")]
    DecryptionFailed,
    #[error("Invalid nonce")]
    InvalidNonce,
    #[error("Invalid ciphertext")]
    InvalidCiphertext,
    #[error("Invalid tag")]
    InvalidTag,
    #[error("Timestamp expired")]
    TimestampExpired,
    #[error("Invalid UTF-8")]
    InvalidUtf8,
}

/// 将 EncryptedPacket 转换为协议格式
impl From<EncryptedPacket> for EncryptedPayload {
    fn from(packet: EncryptedPacket) -> Self {
        EncryptedPayload {
            version: packet.version,
            timestamp: packet.timestamp,
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &packet.nonce),
            ciphertext: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &packet.ciphertext),
            tag: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &packet.tag),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key_pair = EcdhKeyPair::generate();
        let peer_key_pair = EcdhKeyPair::generate();

        let key = compute_shared_secret(&key_pair.private_key, &peer_key_pair.public_key);
        let plaintext = "Hello, World!";

        let encrypted = encrypt(plaintext, &key).expect("Encryption failed");
        let payload: EncryptedPayload = encrypted.into();
        let decrypted = decrypt(&payload, &key).expect("Decryption failed");

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_key_generation() {
        let key_pair = EcdhKeyPair::generate();
        assert_eq!(key_pair.public_key.len(), 65); // SEC1 encoding: 0x04 || 32-byte X || 32-byte Y
        assert_eq!(key_pair.private_key.len(), 32);
    }
}
