//! Ed25519 Authentication for Relay
//!
//! 实现与 relay.yinnho.cn 的 Ed25519 签名认证

use ed25519_dalek::{Signature, Signer, SigningKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

/// Ed25519 签名密钥对
#[derive(Debug, Clone)]
pub struct SigningKeyPair {
    pub public_key: Vec<u8>,
    pub private_key: Vec<u8>,
}

impl SigningKeyPair {
    /// 生成新的密钥对
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key().to_bytes().to_vec();
        let private_key = signing_key.to_bytes().to_vec();

        Self {
            public_key,
            private_key,
        }
    }

    /// 从 DER bytes 恢复密钥对 (PKCS8 格式)
    pub fn from_pkcs8(private_der: &[u8], public_der: &[u8]) -> Option<Self> {
        let signing_key = SigningKey::from_bytes(private_der.try_into().ok()?);
        let public_key = signing_key.verifying_key().to_bytes().to_vec();

        // 验证公钥匹配
        if public_key != public_der {
            return None;
        }

        Some(Self {
            public_key,
            private_key: private_der.to_vec(),
        })
    }

    /// 从原始字节创建密钥对（用于从文件加载）
    pub fn from_bytes(public_key: &[u8], private_key: &[u8]) -> Option<Self> {
        // 验证公钥长度 (32 bytes for Ed25519)
        if public_key.len() != 32 {
            return None;
        }
        // 验证私钥长度 (32 bytes for Ed25519)
        if private_key.len() != 32 {
            return None;
        }
        // 验证密钥对有效
        let private_array: [u8; 32] = match private_key.try_into() {
            Ok(arr) => arr,
            Err(_) => return None,
        };
        let signing_key = SigningKey::from_bytes(&private_array);
        let computed_public_key = signing_key.verifying_key().to_bytes().to_vec();
        if computed_public_key != public_key {
            return None;
        }
        Some(Self {
            public_key: public_key.to_vec(),
            private_key: private_key.to_vec(),
        })
    }

    /// 获取 Base64 编码的公钥
    pub fn public_key_base64(&self) -> String {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &self.public_key)
    }

    /// 获取 Base64 编码的私钥
    pub fn private_key_base64(&self) -> String {
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &self.private_key,
        )
    }
}

/// 对消息进行签名
pub fn sign_message(message: &str, private_key: &[u8]) -> Vec<u8> {
    let signing_key =
        SigningKey::from_bytes(private_key.try_into().expect("Invalid private key length"));
    let signature: Signature = signing_key.sign(message.as_bytes());
    signature.to_bytes().to_vec()
}

/// 验证签名
pub fn verify_signature(message: &str, signature: &[u8], public_key: &[u8]) -> bool {
    use ed25519_dalek::Verifier;
    use ed25519_dalek::VerifyingKey;

    // Convert public_key to VerifyingKey
    let verifying_key = match VerifyingKey::from_bytes(public_key.try_into().unwrap_or(&[0u8; 32]))
    {
        Ok(vk) => vk,
        Err(_) => return false,
    };

    // Convert signature
    let sig_array: [u8; 64] = match signature.try_into() {
        Ok(arr) => arr,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(&sig_array);

    verifying_key.verify(message.as_bytes(), &sig).is_ok()
}

/// 创建认证消息
pub fn create_auth_message(
    carrier_id: &str,
    role: &str,
    private_key: &[u8],
    jwt: Option<String>,
    device_id: Option<String>,
) -> AuthMessageData {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let message = format!("{carrier_id}:{role}:{timestamp}");
    let signature = sign_message(&message, private_key);

    AuthMessageData {
        msg_type: "auth".to_string(),
        carrier_id: carrier_id.to_string(),
        role: role.to_string(),
        timestamp,
        signature: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &signature),
        jwt,
        device_id,
    }
}

/// 认证消息数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMessageData {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub carrier_id: String,
    pub role: String,
    pub timestamp: i64,
    pub signature: String,
    pub jwt: Option<String>,
    pub device_id: Option<String>,
}

impl AuthMessageData {
    /// 创建新的认证消息
    pub fn new(
        carrier_id: String,
        role: String,
        private_key: &[u8],
        jwt: Option<String>,
        device_id: Option<String>,
    ) -> Self {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let message = format!("{carrier_id}:{role}:{timestamp}");
        let signature = sign_message(&message, private_key);

        Self {
            msg_type: "auth".to_string(),
            carrier_id,
            role,
            timestamp,
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &signature,
            ),
            jwt,
            device_id,
        }
    }
}

/// 验证认证消息的签名
pub fn verify_auth_message(
    carrier_id: &str,
    role: &str,
    timestamp: i64,
    signature: &str,
    public_key: &[u8],
) -> bool {
    // 验证时间戳（30秒内有效）
    let now = chrono::Utc::now().timestamp_millis();
    if (now - timestamp).abs() > 30000 {
        tracing::warn!(
            "Auth message timestamp out of range: {} vs {}",
            now,
            timestamp
        );
        return false;
    }

    // 验证签名
    let message = format!("{carrier_id}:{role}:{timestamp}");
    let sig_bytes =
        match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature) {
            Ok(s) => s,
            Err(_) => return false,
        };

    verify_signature(&message, &sig_bytes, public_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let key_pair = SigningKeyPair::generate();
        let message = "test message";

        let signature = sign_message(message, &key_pair.private_key);
        assert!(verify_signature(message, &signature, &key_pair.public_key));
    }

    #[test]
    fn test_create_auth_message() {
        let key_pair = SigningKeyPair::generate();
        let auth = AuthMessageData::new(
            "123".to_string(),
            "carrier".to_string(),
            &key_pair.private_key,
            None,
            None,
        );

        assert_eq!(auth.msg_type, "auth");
        assert_eq!(auth.carrier_id, "123");
        assert_eq!(auth.role, "carrier");
    }
}
