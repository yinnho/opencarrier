//! Hub client — search and install .agx clones from openclone-hub.
//!
//! Adapted from openclone-core/src/hub.rs with API Key authentication.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct SearchResponse {
    templates: Vec<TemplateItem>,
    total: usize,
}

#[derive(Deserialize)]
struct TemplateItem {
    name: String,
    description: String,
    #[allow(dead_code)]
    author: String,
    latest_version: String,
    download_count: i64,
    rating_avg: f64,
}

/// Search templates on Hub. Returns formatted table string.
pub async fn search(hub_url: &str, api_key: &str, query: &str) -> Result<String> {
    let base = hub_url.trim_end_matches('/');
    let url = if query.is_empty() {
        format!("{}/api/templates?limit=20", base)
    } else {
        format!("{}/api/templates?q={}&limit=20", base, urlencoding::encode(query))
    };

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(api_key)
        .send()
        .await
        .context("无法连接 Hub")?;

    if !resp.status().is_success() {
        bail!("Hub 返回错误: {}", resp.status());
    }

    let data: SearchResponse = resp.json().await.context("解析 Hub 响应失败")?;

    if data.templates.is_empty() {
        return Ok("没有找到匹配的模版".to_string());
    }

    let mut out = format!("找到 {} 个模版:\n\n", data.total);
    out.push_str(&format!("{:<25} {:<12} {:<8} {:<6} {}\n", "名称", "版本", "下载", "评分", "描述"));
    out.push_str(&format!("{}\n", "-".repeat(80)));

    for t in &data.templates {
        let desc = if t.description.chars().count() > 30 {
            format!("{}…", t.description.chars().take(30).collect::<String>())
        } else {
            t.description.clone()
        };
        let stars = format_stars(t.rating_avg);
        out.push_str(&format!(
            "{:<25} {:<12} {:<8} {:<6} {}\n",
            t.name, t.latest_version, t.download_count, stars, desc
        ));
    }

    Ok(out)
}

/// Download and install a clone from Hub.
/// Returns the clone name on success.
pub async fn install(
    hub_url: &str,
    api_key: &str,
    name: &str,
    version: Option<&str>,
    workspace_dir: &Path,
    device_id: &str,
) -> Result<String> {
    let base = hub_url.trim_end_matches('/');
    let url = if let Some(v) = version {
        format!("{}/api/templates/{}/download/{}", base, name, v)
    } else {
        format!("{}/api/templates/{}/download", base, name)
    };

    tracing::info!("正在从 Hub 下载 {} ...", name);

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(api_key)
        .header("X-Device-ID", device_id)
        .send()
        .await
        .context("无法连接 Hub")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("下载失败 {}: {} — {}", name, status, body);
    }

    let bytes = resp.bytes().await.context("读取响应失败")?;
    tracing::info!("已下载 {} 字节", bytes.len());

    // Write to temp file, then load via load_agx
    let tmp_dir = std::env::temp_dir().join(format!("opencarrier-hub-{}", std::process::id()));
    if tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    std::fs::create_dir_all(&tmp_dir)?;
    let tmp_file = tmp_dir.join(format!("{}.agx", name));
    std::fs::write(&tmp_file, &bytes)?;

    // Load and install
    let clone_data = crate::load_agx(&tmp_file)?;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let clone_name = clone_data.name.clone();
    crate::install_clone_to_workspace(&clone_data, workspace_dir)?;

    tracing::info!("分身 '{}' 安装完成", clone_name);
    Ok(clone_name)
}

/// Generate or load a persistent device ID.
/// Stored in ~/.opencarrier/device_id as a simple hex string.
pub fn get_or_create_device_id(home_dir: &Path) -> Result<String> {
    let path = home_dir.join("device_id");
    if path.exists() {
        let id = std::fs::read_to_string(&path)?.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    // Generate new: 32 hex chars
    let id = {
        use std::fmt::Write;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("rng");
        let mut hex = String::with_capacity(32);
        for b in &bytes {
            write!(&mut hex, "{:02x}", b).unwrap();
        }
        hex
    };
    std::fs::write(&path, &id)?;
    Ok(id)
}

/// Read API key from the configured env var. Returns error if not set.
pub fn read_api_key(env_var: &str) -> Result<String> {
    std::env::var(env_var).context(format!(
        "API Key 未设置。请设置环境变量 {} (从 Hub 的 Keys 页面获取)",
        env_var
    ))
}

/// Publish (upload) a clone .agx to Hub.
/// Sends JSON with base64-encoded .agx file, matching Hub's PublishPayload format.
pub async fn publish(
    hub_url: &str,
    api_key: &str,
    agx_bytes: &[u8],
    device_id: &str,
    category: Option<&str>,
    visibility: Option<&str>,
) -> Result<String> {
    use base64::Engine;
    let base = hub_url.trim_end_matches('/');
    let url = format!("{}/api/templates", base);

    let file_base64 = base64::engine::general_purpose::STANDARD.encode(agx_bytes);

    let mut payload = serde_json::json!({
        "file_base64": file_base64,
    });
    if let Some(cat) = category {
        payload["category"] = serde_json::Value::String(cat.to_string());
    }
    if let Some(vis) = visibility {
        payload["visibility"] = serde_json::Value::String(vis.to_string());
    }

    tracing::info!("正在发布到 Hub ({} bytes / {:.1} KB)...", agx_bytes.len(), agx_bytes.len() as f64 / 1024.0);

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .header("X-Device-ID", device_id)
        .json(&payload)
        .send()
        .await
        .context("无法连接 Hub")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("发布失败: {} — {}", status, body);
    }

    let body: serde_json::Value = resp.json().await.context("解析 Hub 响应失败")?;
    let name = body["name"].as_str().unwrap_or("unknown");
    let version = body["version"].as_str().unwrap_or("unknown");
    let status = body["status"].as_str().unwrap_or("unknown");
    tracing::info!("发布成功: {} v{} ({})", name, version, status);
    Ok(name.to_string())
}

fn format_stars(avg: f64) -> String {
    let full = (avg / 1.0).round() as i32;
    (0..5)
        .map(|i| if i < full { "★" } else { "☆" })
        .collect::<Vec<_>>()
        .join("")
}
