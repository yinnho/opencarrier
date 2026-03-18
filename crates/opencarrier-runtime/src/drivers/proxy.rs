//! Cloud Proxy LLM Driver
//!
//! 通过云端代理调用 LLM，API Key 只存在于云端。
//! 这与 yingheclient 的 ProxyLLMClient 功能相同。
//!
//! 使用方式：
//! 1. 先绑定：调用 `yinghe bind` 或使用 `CarrierCloudClient` 进行绑定
//! 2. 绑定后 token 自动保存，LLM 调用会自动使用该 token

use async_trait::async_trait;
use opencarrier_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use opencarrier_types::tool::{ToolCall, ToolDefinition};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;

use crate::cloud_client::CarrierCloudClient;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};

/// Default cloud API base URL
const DEFAULT_CLOUD_API_URL: &str = "https://carrier.yinnho.cn";

/// LLM Proxy request format (sent to cloud)
#[derive(Debug, Serialize)]
struct ProxyRequest {
    /// Model identifier (cloud may override)
    model: String,
    /// Messages in OpenAI format
    messages: Vec<ProxyMessage>,
    /// Maximum tokens to generate
    max_tokens: u32,
    /// Sampling temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Modality hint for endpoint selection
    #[serde(skip_serializing_if = "Option::is_none")]
    modality: Option<String>,
    /// Tool definitions
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    /// Tool choice strategy
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

/// Message in OpenAI-compatible format
#[derive(Debug, Serialize)]
struct ProxyMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<ProxyContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ProxyToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// Content can be string or array of parts
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ProxyContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// Content part for multimodal
#[derive(Debug, Serialize)]
struct ContentPart {
    #[serde(rename = "type")]
    part_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<ImageUrl>,
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// Tool call in response
#[derive(Debug, Serialize, Deserialize)]
struct ProxyToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ProxyFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProxyFunction {
    name: String,
    arguments: String,
}

/// OpenAI-compatible response from cloud proxy
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProxyResponse {
    id: String,
    model: String,
    choices: Vec<ProxyChoice>,
    #[serde(default)]
    usage: Option<ProxyUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProxyChoice {
    index: u32,
    message: ProxyResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProxyResponseMessage {
    role: String,
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ProxyToolCall>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProxyUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// Endpoint info from cloud
#[derive(Debug, Deserialize)]
pub struct ProxyEndpoint {
    pub id: String,
    pub provider: String,
    pub model: String,
}

/// Cloud Proxy LLM Driver
///
/// All LLM requests go through the cloud proxy. API keys are stored
/// in the cloud, not locally. The carrier needs to be bound first.
///
/// # 绑定流程
/// 1. 运行 `yinghe bind` 或调用 `CarrierCloudClient::perform_binding()`
/// 2. 在 App 上输入配对码完成绑定
/// 3. 绑定成功后，token 自动保存，ProxyDriver 自动使用
pub struct ProxyDriver {
    client: Client,
    cloud_url: String,
    /// 引用共享的云端客户端
    cloud_client: Arc<CarrierCloudClient>,
    /// 缓存的 token（避免频繁读取文件）
    cached_token: Arc<RwLock<Option<String>>>,
    default_modality: String,
    endpoints: Vec<ProxyEndpoint>,
}

impl ProxyDriver {
    /// Create a new proxy driver with a cloud client
    ///
    /// The cloud client handles binding and token management.
    /// Make sure to bind first using `CarrierCloudClient::perform_binding()`
    /// or the CLI `yinghe bind` command.
    pub fn new(cloud_client: Arc<CarrierCloudClient>) -> Self {
        let cloud_url = std::env::var("OPENCARRIER_CLOUD_URL")
            .unwrap_or_else(|_| DEFAULT_CLOUD_API_URL.to_string());

        Self {
            client: Client::new(),
            cloud_url,
            cloud_client,
            cached_token: Arc::new(RwLock::new(None)),
            default_modality: "chat".to_string(),
            endpoints: Vec::new(),
        }
    }

    /// Create a proxy driver from existing binding info
    ///
    /// Convenience method that creates a CarrierCloudClient internally.
    pub async fn from_binding(cloud_url: Option<String>) -> Result<Self, LlmError> {
        let cloud_client = Arc::new(CarrierCloudClient::new(cloud_url));

        // 检查是否已绑定
        if !cloud_client.is_bound().await {
            return Err(LlmError::Config(
                "Carrier not bound. Run 'yinghe bind' first or set OPENCARRIER_TOKEN env var"
                    .to_string(),
            ));
        }

        Ok(Self::new(cloud_client))
    }

    /// Get the auth token (from cache or binding file)
    async fn get_token(&self) -> Result<String, LlmError> {
        // 先检查缓存
        {
            let cached = self.cached_token.read().await;
            if let Some(ref token) = *cached {
                return Ok(token.clone());
            }
        }

        // 从 cloud_client 获取
        let token = self.cloud_client.get_token().await.ok_or_else(|| {
            LlmError::Config("Carrier not bound. Run 'yinghe bind' first.".to_string())
        })?;

        // 缓存 token
        {
            let mut cached = self.cached_token.write().await;
            *cached = Some(token.clone());
        }

        Ok(token)
    }

    /// Initialize - load endpoints from cloud
    pub async fn initialize(&mut self) -> Result<(), LlmError> {
        let token = self.get_token().await?;

        // Fetch endpoints from cloud
        let url = format!("{}/llm/endpoints", self.cloud_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| LlmError::Http(format!("Failed to fetch endpoints: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status,
                message: format!("Failed to fetch endpoints: {}", text),
            });
        }

        #[derive(Deserialize)]
        struct EndpointsResponse {
            endpoints: Vec<ProxyEndpoint>,
        }

        let data: EndpointsResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(format!("Failed to parse endpoints: {}", e)))?;

        self.endpoints = data.endpoints;
        tracing::info!("Loaded {} proxy endpoints", self.endpoints.len());

        // Fetch modalities
        let url = format!("{}/llm/modalities", self.cloud_url);
        if let Ok(resp) = self.client.get(&url).bearer_auth(&token).send().await {
            if resp.status().is_success() {
                tracing::info!("Loaded modalities from cloud");
            }
        }

        Ok(())
    }

    /// Set default modality
    pub fn set_default_modality(&mut self, modality: String) {
        self.default_modality = modality;
    }

    /// Get available endpoints
    pub fn get_endpoints(&self) -> &[ProxyEndpoint] {
        &self.endpoints
    }

    /// Get the cloud client (for binding operations)
    pub fn cloud_client(&self) -> Arc<CarrierCloudClient> {
        self.cloud_client.clone()
    }

    /// Convert internal messages to proxy format (OpenAI-compatible)
    fn convert_messages(messages: &[Message]) -> Vec<ProxyMessage> {
        let mut proxy_messages = Vec::new();

        for msg in messages {
            match (msg.role, &msg.content) {
                // Simple text messages
                (Role::System, MessageContent::Text(text)) => {
                    proxy_messages.push(ProxyMessage {
                        role: "system".to_string(),
                        content: Some(ProxyContent::Text(text.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                (Role::User, MessageContent::Text(text)) => {
                    proxy_messages.push(ProxyMessage {
                        role: "user".to_string(),
                        content: Some(ProxyContent::Text(text.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                (Role::Assistant, MessageContent::Text(text)) => {
                    proxy_messages.push(ProxyMessage {
                        role: "assistant".to_string(),
                        content: Some(ProxyContent::Text(text.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }

                // User with blocks (text + images)
                (Role::User, MessageContent::Blocks(blocks)) => {
                    let mut parts = Vec::new();
                    let mut has_tool_results = false;

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => {
                                parts.push(ContentPart {
                                    part_type: "text".to_string(),
                                    text: Some(text.clone()),
                                    image_url: None,
                                });
                            }
                            ContentBlock::Image { media_type, data } => {
                                let url = format!("data:{};base64,{}", media_type, data);
                                parts.push(ContentPart {
                                    part_type: "image_url".to_string(),
                                    text: None,
                                    image_url: Some(ImageUrl { url, detail: None }),
                                });
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                                ..
                            } => {
                                has_tool_results = true;
                                // Tool results go as separate "tool" role messages
                                proxy_messages.push(ProxyMessage {
                                    role: "tool".to_string(),
                                    content: Some(ProxyContent::Text(if *is_error {
                                        format!("Error: {}", content)
                                    } else {
                                        content.clone()
                                    })),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                });
                            }
                            _ => {}
                        }
                    }

                    // Add user message with parts if not just tool results
                    if !parts.is_empty() && !has_tool_results {
                        proxy_messages.push(ProxyMessage {
                            role: "user".to_string(),
                            content: Some(ProxyContent::Parts(parts)),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }

                // Assistant with blocks (text + tool calls + thinking)
                (Role::Assistant, MessageContent::Blocks(blocks)) => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => {
                                text_parts.push(text.clone());
                            }
                            ContentBlock::ToolUse {
                                id, name, input, ..
                            } => {
                                tool_calls.push(ProxyToolCall {
                                    id: id.clone(),
                                    call_type: "function".to_string(),
                                    function: ProxyFunction {
                                        name: name.clone(),
                                        arguments: input.to_string(),
                                    },
                                });
                            }
                            _ => {}
                        }
                    }

                    let has_tool_calls = !tool_calls.is_empty();
                    proxy_messages.push(ProxyMessage {
                        role: "assistant".to_string(),
                        content: if text_parts.is_empty() {
                            if has_tool_calls {
                                Some(ProxyContent::Text(String::new()))
                            } else {
                                None
                            }
                        } else {
                            Some(ProxyContent::Text(text_parts.join("")))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    });
                }

                // System with blocks (rare, treat as text join)
                (Role::System, MessageContent::Blocks(blocks)) => {
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    proxy_messages.push(ProxyMessage {
                        role: "system".to_string(),
                        content: Some(ProxyContent::Text(text)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        proxy_messages
    }

    /// Convert tools to proxy format
    fn convert_tools(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }
}

#[async_trait]
impl LlmDriver for ProxyDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let token = self.get_token().await?;

        // Get default model from first endpoint
        let default_model = self
            .endpoints
            .first()
            .map(|e| e.model.clone())
            .unwrap_or_else(|| request.model.clone());

        // Build proxy request
        let messages = Self::convert_messages(&request.messages);

        let proxy_req = ProxyRequest {
            model: default_model,
            messages,
            max_tokens: request.max_tokens,
            temperature: Some(request.temperature),
            modality: Some(self.default_modality.clone()),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(&request.tools))
            },
            tool_choice: if request.tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        // Call cloud proxy
        let url = format!("{}/llm/chat", self.cloud_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&proxy_req)
            .send()
            .await
            .map_err(|e| LlmError::Http(format!("Proxy request failed: {}", e)))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();

            // 如果是认证错误，清除缓存的 token
            if status == 401 {
                let mut cached = self.cached_token.write().await;
                *cached = None;
                return Err(LlmError::Config(
                    "Token expired or invalid. Run 'yinghe bind' to re-authenticate.".to_string(),
                ));
            }

            return Err(LlmError::Api {
                status,
                message: format!("Proxy API error: {}", text),
            });
        }

        let proxy_resp: ProxyResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(format!("Failed to parse response: {}", e)))?;

        // Convert response
        let choice = proxy_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Parse("No choices in response".to_string()))?;

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        // Parse tool calls
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                let input: serde_json::Value = tc
                    .function
                    .arguments
                    .parse()
                    .unwrap_or(serde_json::Value::Null);
                ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    input,
                }
            })
            .collect();

        // Build content blocks
        let mut content: Vec<ContentBlock> = Vec::new();

        // Add reasoning content if present
        if let Some(reasoning) = choice.message.reasoning_content {
            if !reasoning.is_empty() {
                content.push(ContentBlock::Thinking {
                    thinking: reasoning,
                });
            }
        }

        // Add main content
        if let Some(text) = choice.message.content {
            if !text.is_empty() {
                content.push(ContentBlock::Text {
                    text,
                    provider_metadata: None,
                });
            }
        }

        let usage = proxy_resp
            .usage
            .map(|u| TokenUsage {
                input_tokens: u.prompt_tokens as u64,
                output_tokens: u.completion_tokens as u64,
            })
            .unwrap_or(TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
            });

        Ok(CompletionResponse {
            content,
            stop_reason,
            tool_calls,
            usage,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        // For now, fall back to non-streaming
        // TODO: Implement true streaming when cloud supports it
        let response = self.complete(request).await?;

        // Extract text content
        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Send text delta
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::TextDelta { text }).await;
        }

        // Send completion event
        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_request_serialization() {
        let req = ProxyRequest {
            model: "gpt-4".to_string(),
            messages: vec![ProxyMessage {
                role: "user".to_string(),
                content: Some(ProxyContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: 100,
            temperature: Some(0.7),
            modality: Some("chat".to_string()),
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("gpt-4"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_convert_simple_messages() {
        let messages = vec![
            Message::system("You are helpful."),
            Message::user("Hello"),
            Message::assistant("Hi there!"),
        ];

        let converted = ProxyDriver::convert_messages(&messages);

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[2].role, "assistant");
    }
}
