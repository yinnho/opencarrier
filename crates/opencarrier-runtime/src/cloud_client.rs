//! Cloud API Client for OpenCarrier
//!
//! 与应合云端 API 交互的客户端，实现：
//! - 配对码绑定流程
//! - LLM 代理调用
//! - 载体状态上报
//! - Relay WebSocket 连接
//!
//! 这与 yingheclient 的 CarrierClient 功能相同。

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use ying_relay::{RelayClient, RelayEvent, SigningKeyPair};

/// Default cloud API base URL
pub const DEFAULT_CLOUD_API_URL: &str = "https://carrier.yinnho.cn";

/// 配置文件中存储的绑定信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingInfo {
    pub token: String,
    pub carrier_id: u64,
    pub device_id: String,
}

/// 签名密钥文件格式（hex 编码，与 yingheclient 兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SigningKeyData {
    public_key: String,  // hex 编码
    private_key: String, // hex 编码
}

/// 配对码响应
#[derive(Debug, Deserialize)]
pub struct PairingCodeResponse {
    pub pairing_code: String,
    pub device_id: String,
    pub expires_in: u64,
}

/// 绑定状态检查响应
#[derive(Debug, Deserialize)]
pub struct BindingStatusResponse {
    pub bound: bool,
    pub carrier_id: Option<u64>,
    pub user_id: Option<u64>,
    pub token: Option<String>,
}

/// LLM Endpoints 响应
#[derive(Debug, Deserialize)]
pub struct EndpointsResponse {
    pub endpoints: Vec<EndpointInfo>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EndpointInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
}

/// 云端 API 错误
#[derive(Debug)]
pub enum CloudError {
    Http(String),
    Api { status: u16, message: String },
    NotBound,
    Parse(String),
    Io(String),
}

impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudError::Http(msg) => write!(f, "HTTP error: {}", msg),
            CloudError::Api { status, message } => write!(f, "API error ({}): {}", status, message),
            CloudError::NotBound => write!(f, "Carrier not bound to cloud"),
            CloudError::Parse(msg) => write!(f, "Parse error: {}", msg),
            CloudError::Io(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for CloudError {}

/// Cloud API Client
pub struct CarrierCloudClient {
    client: Client,
    cloud_url: String,
    /// 缓存的绑定信息（内存中）
    binding: Arc<RwLock<Option<BindingInfo>>>,
    /// 配置文件路径
    config_path: PathBuf,
    /// Ed25519 签名密钥对（用于 Relay 认证）
    signing_key_pair: Arc<RwLock<Option<SigningKeyPair>>>,
    /// Relay WebSocket 客户端
    relay_client: Arc<RwLock<Option<RelayClient>>>,
}

impl CarrierCloudClient {
    /// Create a new cloud client
    pub fn new(cloud_url: Option<String>) -> Self {
        let cloud_url = cloud_url.unwrap_or_else(|| {
            std::env::var("OPENCARRIER_CLOUD_URL")
                .unwrap_or_else(|_| DEFAULT_CLOUD_API_URL.to_string())
        });

        let config_path = Self::get_config_path();

        Self {
            client: Client::new(),
            cloud_url,
            binding: Arc::new(RwLock::new(None)),
            config_path,
            signing_key_pair: Arc::new(RwLock::new(None)),
            relay_client: Arc::new(RwLock::new(None)),
        }
    }

    fn get_config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".opencarrier").join("binding.json")
    }

    /// 从配置文件加载绑定信息
    pub async fn load_binding(&self) -> Option<BindingInfo> {
        // 先检查内存缓存
        {
            let binding = self.binding.read().await;
            if binding.is_some() {
                return binding.clone();
            }
        }

        // 从文件加载
        if let Ok(content) = tokio::fs::read_to_string(&self.config_path).await {
            if let Ok(info) = serde_json::from_str::<BindingInfo>(&content) {
                info!(carrier_id = info.carrier_id, "Loaded binding from file");
                let mut binding = self.binding.write().await;
                *binding = Some(info.clone());
                return Some(info);
            }
        }

        // 尝试从环境变量加载（兼容旧方式）
        if let (Ok(token), Ok(carrier_id)) = (
            std::env::var("OPENCARRIER_TOKEN"),
            std::env::var("OPENCARRIER_CARRIER_ID"),
        ) {
            if let Ok(carrier_id) = carrier_id.parse::<u64>() {
                let device_id = std::env::var("OPENCARRIER_DEVICE_ID")
                    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
                let info = BindingInfo {
                    token,
                    carrier_id,
                    device_id: device_id.clone(),
                };
                info!("Loaded binding from environment variables");
                let mut binding = self.binding.write().await;
                *binding = Some(info.clone());
                return Some(info);
            }
        }

        None
    }

    /// 保存绑定信息到配置文件
    async fn save_binding(&self, info: &BindingInfo) -> Result<(), CloudError> {
        // 确保目录存在
        if let Some(parent) = self.config_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Err(CloudError::Io(format!(
                    "Failed to create config dir: {}",
                    e
                )));
            }
        }

        let content = serde_json::to_string_pretty(info)
            .map_err(|e| CloudError::Parse(format!("Failed to serialize binding: {}", e)))?;

        tokio::fs::write(&self.config_path, &content)
            .await
            .map_err(|e| CloudError::Io(format!("Failed to write binding file: {}", e)))?;

        // 设置文件权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&self.config_path, perms);
        }

        info!(path = %self.config_path.display(), "Saved binding to file");

        // 更新内存缓存
        let mut binding = self.binding.write().await;
        *binding = Some(info.clone());

        Ok(())
    }

    /// 清除绑定信息
    pub async fn clear_binding(&self) -> Result<(), CloudError> {
        // 删除文件
        if self.config_path.exists() {
            tokio::fs::remove_file(&self.config_path)
                .await
                .map_err(|e| CloudError::Io(format!("Failed to remove binding file: {}", e)))?;
        }

        // 清除内存缓存
        let mut binding = self.binding.write().await;
        *binding = None;

        info!("Cleared binding info");
        Ok(())
    }

    /// 获取当前绑定信息
    pub async fn get_binding(&self) -> Option<BindingInfo> {
        self.load_binding().await
    }

    /// 检查是否已绑定
    pub async fn is_bound(&self) -> bool {
        self.load_binding()
            .await
            .map(|b| b.carrier_id != 0 && !b.token.is_empty())
            .unwrap_or(false)
    }

    /// 获取认证 token
    pub async fn get_token(&self) -> Option<String> {
        self.load_binding()
            .await
            .filter(|b| !b.token.is_empty())
            .map(|b| b.token)
    }

    /// 创建配对码（用于 App 扫码绑定）
    pub async fn create_pairing_code(&self) -> Result<PairingCodeResponse, CloudError> {
        // 先获取或创建设备ID并保存（这样在轮询时能保持一致）
        let device_id = self.get_or_create_device_id().await;
        let info = BindingInfo {
            token: String::new(),
            carrier_id: 0,
            device_id: device_id.clone(),
        };
        self.save_binding(&info).await?;

        let url = format!("{}/carrier/pairing", self.cloud_url);
        let body = serde_json::json!({
            "device_id": device_id,
            "device_info": self.get_device_info(),
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to request pairing code: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to create pairing code: {}", text),
            });
        }

        let result: PairingCodeResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::Parse(format!("Failed to parse pairing response: {}", e)))?;

        info!(code = %result.pairing_code, expires_in = result.expires_in, device_id = %result.device_id, "Created pairing code");

        // 注意：result.device_id 可能与发送的 device_id 不同（服务器可能返回新的）
        // 如果不同，需要更新保存
        if result.device_id != device_id {
            let info = BindingInfo {
                token: String::new(),
                carrier_id: 0,
                device_id: result.device_id.clone(),
            };
            self.save_binding(&info).await?;
            info!(device_id = %result.device_id, "Updated device_id from server");
        }

        Ok(result)
    }

    /// 轮询检查绑定状态
    pub async fn check_binding(&self) -> Result<BindingStatusResponse, CloudError> {
        let device_id = self
            .load_binding()
            .await
            .map(|b| b.device_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let url = format!("{}/carrier/check-binding", self.cloud_url);
        let body = serde_json::json!({
            "device_id": device_id,
            "device_info": self.get_device_info(),
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to check binding: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to check binding: {}", text),
            });
        }

        resp.json()
            .await
            .map_err(|e| CloudError::Parse(format!("Failed to parse binding status: {}", e)))
    }

    /// 等待绑定完成（轮询）
    /// 返回绑定信息
    pub async fn wait_for_binding(
        &self,
        pairing_code: &str,
        timeout_secs: u64,
    ) -> Result<BindingInfo, CloudError> {
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(3);
        let timeout = std::time::Duration::from_secs(timeout_secs);

        info!(code = %pairing_code, timeout = timeout_secs, "Waiting for binding...");

        while start.elapsed() < timeout {
            let status = self.check_binding().await?;

            if status.bound {
                // 检查 carrier_id 是否有效（必须 > 0）
                if let (Some(token), Some(carrier_id)) = (status.token, status.carrier_id) {
                    if carrier_id == 0 {
                        warn!("Server returned carrier_id=0, binding not actually complete");
                    } else {
                        let device_id = self
                            .load_binding()
                            .await
                            .map(|b| b.device_id)
                            .unwrap_or_default();

                        let info = BindingInfo {
                            token,
                            carrier_id,
                            device_id,
                        };

                        // 保存到文件
                        self.save_binding(&info).await?;

                        info!(carrier_id = info.carrier_id, "Binding completed!");
                        return Ok(info);
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }

        Err(CloudError::NotBound)
    }

    /// 获取 LLM endpoints
    pub async fn get_llm_endpoints(&self) -> Result<Vec<EndpointInfo>, CloudError> {
        let token = self.get_token().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/llm/endpoints", self.cloud_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to get endpoints: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to get endpoints: {}", text),
            });
        }

        let result: EndpointsResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::Parse(format!("Failed to parse endpoints: {}", e)))?;

        Ok(result.endpoints)
    }

    /// 调用 LLM 代理
    pub async fn call_llm_proxy(
        &self,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, CloudError> {
        let token = self.get_token().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/llm/chat", self.cloud_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("LLM proxy request failed: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("LLM proxy error: {}", text),
            });
        }

        resp.json()
            .await
            .map_err(|e| CloudError::Parse(format!("Failed to parse LLM response: {}", e)))
    }

    /// 上报载体在线状态
    pub async fn report_online(&self) -> Result<(), CloudError> {
        let binding = self.load_binding().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/relay/carrier/online", self.cloud_url);
        let body = serde_json::json!({
            "carrier_id": binding.carrier_id,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&binding.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to report online: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to report online: {}", text),
            });
        }

        debug!(carrier_id = binding.carrier_id, "Reported online");
        Ok(())
    }

    /// 上报载体离线状态
    pub async fn report_offline(&self) -> Result<(), CloudError> {
        let binding = self.load_binding().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/relay/carrier/offline", self.cloud_url);
        let body = serde_json::json!({
            "carrier_id": binding.carrier_id,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&binding.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to report offline: {}", e)))?;

        if !resp.status().is_success() {
            warn!("Failed to report offline status");
        }

        Ok(())
    }

    /// 注册 ECDH 公钥到云端
    pub async fn register_carrier_key(&self, carrier_id: u64, public_key: &str) -> Result<(), CloudError> {
        let binding = self.load_binding().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/relay/carrier/key", self.cloud_url);
        let body = serde_json::json!({
            "carrier_id": carrier_id,
            "public_key": public_key,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&binding.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to register carrier key: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to register carrier key: {}", text),
            });
        }

        info!(carrier_id = carrier_id, "ECDH public key registered");
        Ok(())
    }

    /// 注册 Ed25519 公钥到云端
    pub async fn register_ed25519_key(&self, carrier_id: u64, ed25519_public_key_hex: &str) -> Result<(), CloudError> {
        let binding = self.load_binding().await.ok_or(CloudError::NotBound)?;

        let url = format!("{}/relay/carrier/ed25519key", self.cloud_url);
        let body = serde_json::json!({
            "carrier_id": carrier_id,
            "ed25519_public_key": ed25519_public_key_hex,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&binding.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("Failed to register Ed25519 key: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status,
                message: format!("Failed to register Ed25519 key: {}", text),
            });
        }

        info!(carrier_id = carrier_id, "Ed25519 public key registered");
        Ok(())
    }

    /// 获取或创建 device ID（基于机器指纹，重装后不变）
    async fn get_or_create_device_id(&self) -> String {
        // 优先从 binding 文件加载（保持兼容性）
        if let Some(binding) = self.load_binding().await {
            if !binding.device_id.is_empty() {
                return binding.device_id;
            }
        }

        // 生成基于机器指纹的 device_id
        Self::generate_machine_fingerprint()
    }

    /// 生成机器指纹（跨平台稳定）
    fn generate_machine_fingerprint() -> String {
        use sha2::{Digest, Sha256};

        // 尝试获取系统机器 ID
        let machine_id = Self::get_system_machine_id()
            .or_else(|| {
                // 备用：使用主机名 + 用户名 + OS 信息
                let hostname = std::env::var("HOSTNAME")
                    .or_else(|_| std::env::var("COMPUTERNAME"))
                    .unwrap_or_else(|_| "unknown".to_string());
                let username = std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "unknown".to_string());
                Some(format!("{}-{}-{}-{}", hostname, username, std::env::consts::OS, std::env::consts::ARCH))
            })
            .unwrap_or_else(|| "opencarrier-device".to_string());

        // 使用 SHA256 哈希并取前 16 字节作为 device_id
        let mut hasher = Sha256::new();
        hasher.update(machine_id.as_bytes());
        let result = hasher.finalize();
        hex::encode(&result[..16]) // 32 字符的十六进制字符串
    }

    /// 获取系统机器 ID（平台特定）
    #[cfg(target_os = "macos")]
    fn get_system_machine_id() -> Option<String> {
        use std::process::Command;
        // macOS: 使用 IOPlatformUUID
        let output = Command::new("ioreg")
            .args(&["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("IOPlatformUUID") {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 4 {
                    return Some(parts[3].to_string());
                }
            }
        }
        None
    }

    /// 获取系统机器 ID（Linux）
    #[cfg(target_os = "linux")]
    fn get_system_machine_id() -> Option<String> {
        use std::fs;
        // Linux: 读取 /etc/machine-id 或 /var/lib/dbus/machine-id
        fs::read_to_string("/etc/machine-id")
            .or_else(|_| fs::read_to_string("/var/lib/dbus/machine-id"))
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// 获取系统机器 ID（Windows）
    #[cfg(target_os = "windows")]
    fn get_system_machine_id() -> Option<String> {
        use std::process::Command;
        // Windows: 使用 wmic 获取 MachineGuid
        let output = Command::new("wmic")
            .args(&["csproduct", "get", "uuid"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().nth(1).map(|s| s.trim().to_string())
    }

    /// 获取设备信息
    fn get_device_info(&self) -> serde_json::Value {
        // 尝试获取主机名
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME")) // Windows
            .unwrap_or_else(|_| "unknown".to_string());

        serde_json::json!({
            "hostname": hostname,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    // ========== Relay 连接管理 ==========

    /// 获取签名密钥文件路径
    fn get_signing_key_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".opencarrier").join("signing_key.json")
    }

    /// 保存签名密钥对到文件
    async fn save_signing_key_pair(&self, key_pair: &SigningKeyPair) -> Result<(), CloudError> {
        let path = Self::get_signing_key_path();

        // 确保目录存在
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Err(CloudError::Io(format!("Failed to create config dir: {}", e)));
            }
        }

        // 保存为 JSON（hex 格式，与 yingheclient 兼容）
        let data = SigningKeyData {
            public_key: hex::encode(&key_pair.public_key),
            private_key: hex::encode(&key_pair.private_key),
        };

        let content = serde_json::to_string_pretty(&data)
            .map_err(|e| CloudError::Parse(format!("Failed to serialize signing key: {}", e)))?;

        tokio::fs::write(&path, &content)
            .await
            .map_err(|e| CloudError::Io(format!("Failed to write signing key file: {}", e)))?;

        // 设置文件权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&path, perms);
        }

        info!(path = %path.display(), "Saved signing key to file");
        Ok(())
    }

    /// 从文件加载签名密钥对
    async fn load_signing_key_pair() -> Option<SigningKeyPair> {
        let path = Self::get_signing_key_path();

        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Ok(data) = serde_json::from_str::<SigningKeyData>(&content) {
                // 从 hex 解码
                if let (Ok(public_key), Ok(private_key)) = (
                    hex::decode(&data.public_key),
                    hex::decode(&data.private_key),
                ) {
                    info!("Loaded signing key from file");
                    // from_bytes 返回 Option，直接返回
                    return SigningKeyPair::from_bytes(&public_key, &private_key);
                }
            }
        }
        None
    }

    /// 获取或生成 Ed25519 签名密钥对（带持久化）
    pub async fn get_or_create_signing_key_pair(&self) -> SigningKeyPair {
        // 先检查内存缓存
        {
            let guard = self.signing_key_pair.read().await;
            if let Some(ref kp) = *guard {
                return kp.clone();
            }
        }

        // 尝试从文件加载
        if let Some(kp) = Self::load_signing_key_pair().await {
            let mut guard = self.signing_key_pair.write().await;
            *guard = Some(kp.clone());
            return kp;
        }

        // 生成新的
        let kp = SigningKeyPair::generate();

        // 保存到文件
        if let Err(e) = self.save_signing_key_pair(&kp).await {
            error!("Failed to save signing key: {}", e);
        }

        let mut guard = self.signing_key_pair.write().await;
        *guard = Some(kp.clone());
        kp
    }

    /// 获取 Ed25519 公钥（Base64 编码）
    pub async fn get_signing_public_key_base64(&self) -> String {
        let key_pair = self.get_or_create_signing_key_pair().await;
        key_pair.public_key_base64()
    }

    /// 获取 Ed25519 公钥（Hex 编码，用于注册到云端）
    pub async fn get_signing_public_key_hex(&self) -> String {
        let key_pair = self.get_or_create_signing_key_pair().await;
        hex::encode(&key_pair.public_key)
    }

    /// 启动 Relay WebSocket 连接（使用 Arc<Self> 调用）
    pub async fn start_relay(self: Arc<Self>) -> Result<(), CloudError> {
        let binding = self.load_binding().await.ok_or(CloudError::NotBound)?;

        // 获取或创建签名密钥对（持久化的）
        let signing_key_pair = self.get_or_create_signing_key_pair().await;

        // 提前获取 Ed25519 公钥 hex（signing_key_pair 会被 move 到 RelayClient）
        let ed25519_public_key_hex = hex::encode(&signing_key_pair.public_key);

        // 1. 创建 Relay 客户端（会生成 ECDH 密钥对）
        let mut relay = RelayClient::new(
            binding.carrier_id,
            signing_key_pair,
            Some(binding.token.clone()),
            Some(binding.device_id.clone()),
        );

        // 2. 注册 ECDH 公钥到云端（App 绑定需要这个公钥）
        // 使用 SPKI DER 格式（与 yingheclient 一致）
        let ecdh_public_key_spki_der = relay.get_ecdh_public_key_spki_der();
        let ecdh_public_key_base64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &ecdh_public_key_spki_der,
        );
        if let Err(e) = self.register_carrier_key(binding.carrier_id, &ecdh_public_key_base64).await {
            warn!("Failed to register ECDH key: {}", e);
            // 不阻塞，继续尝试
        }

        // 3. 注册 Ed25519 公钥到云端（hex 格式）
        // 注意：这个端点在 yingheyun 中不存在，但 yingheclient 也调用它
        if let Err(e) = self.register_ed25519_key(binding.carrier_id, &ed25519_public_key_hex).await {
            warn!("Failed to register Ed25519 key: {}", e);
            // 不阻塞，继续尝试
        }

        // 4. 上报在线状态
        if let Err(e) = self.report_online().await {
            warn!("Failed to report online: {}", e);
            // 不阻塞，继续
        }

        // 5. 获取事件接收器（需要在连接前获取）
        let event_rx = relay.take_event_receiver();

        // 克隆需要在后台任务中使用的数据
        let binding_clone = binding.clone();
        let client_clone = self.client.clone();
        let cloud_url_clone = self.cloud_url.clone();
        let relay_client_arc = self.relay_client.clone();
        let self_arc = self.clone();

        // 在后台启动事件处理任务
        tokio::spawn(async move {
            let mut event_rx = event_rx;
            while let Some(event) = event_rx.recv().await {
                match event {
                    RelayEvent::Connected => {
                        info!("Relay connected");
                        // 报告在线状态
                        let url = format!("{}/relay/carrier/online", cloud_url_clone);
                        let body = serde_json::json!({
                            "carrier_id": binding_clone.carrier_id,
                        });
                        if let Err(e) = client_clone
                            .post(&url)
                            .bearer_auth(&binding_clone.token)
                            .json(&body)
                            .send()
                            .await
                        {
                            error!("Failed to report online: {}", e);
                        }
                    }
                    RelayEvent::Disconnected => {
                        info!("Relay disconnected");
                    }
                    RelayEvent::PeerConnected { carrier_id } => {
                        info!("Peer connected: {}", carrier_id);
                    }
                    RelayEvent::PeerDisconnected { message } => {
                        info!("Peer disconnected: {:?}", message);
                    }
                    RelayEvent::JwtRefreshed { jwt } => {
                        info!("JWT refreshed");
                        // 更新本地 token
                        if let Some(mut binding) = self_arc.binding.write().await.take() {
                            binding.token = jwt;
                            // 保存到文件
                            let _ = self_arc.save_binding(&binding).await;
                        }
                    }
                    RelayEvent::Message(msg) => {
                        info!("Received relay message: {:?}", msg);
                    }
                    RelayEvent::Error(err) => {
                        error!("Relay error: {}", err);
                    }
                }
            }
        });

        // 先连接
        relay.connect().await.map_err(|e| CloudError::Http(e.to_string()))?;

        // 存储 relay 客户端
        let mut client = relay_client_arc.write().await;
        *client = Some(relay);

        info!("Relay connection started for carrier {}", binding.carrier_id);
        Ok(())
    }

    /// 停止 Relay 连接
    pub async fn stop_relay(&mut self) {
        // 报告离线
        let _ = self.report_offline().await;

        // 断开连接
        {
            let mut client = self.relay_client.write().await;
            if let Some(ref mut r) = *client {
                r.disconnect().await;
            }
            *client = None;
        }

        info!("Relay connection stopped");
    }

    /// 检查 Relay 是否已连接
    pub async fn is_relay_connected(&self) -> bool {
        let client = self.relay_client.read().await;
        match client.as_ref() {
            Some(c) => c.is_connected().await,
            None => false,
        }
    }
}

/// 执行绑定流程（生成配对码并等待）
pub async fn perform_binding(client: &CarrierCloudClient) -> Result<BindingInfo, CloudError> {
    // 生成配对码
    let pairing = client.create_pairing_code().await?;

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║                    载体绑定                                ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║   配对码: {:<46} ║", pairing.pairing_code);
    println!(
        "║   有效期: {:<46} ║",
        format!("{} 分钟", pairing.expires_in / 60)
    );
    println!("║                                                            ║");
    println!("║   请在 App 上输入此配对码进行绑定                          ║");
    println!("║                                                            ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    println!("等待绑定...");

    // 等待绑定完成
    let result = client
        .wait_for_binding(&pairing.pairing_code, pairing.expires_in)
        .await?;

    println!("✓ 绑定成功！载体 ID: {}", result.carrier_id);

    Ok(result)
}
