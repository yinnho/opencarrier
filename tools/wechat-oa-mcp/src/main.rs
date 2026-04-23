//! wechat-oa-mcp — WeChat Official Account MCP Server
//!
//! Exposes draft management, media upload, and publishing tools via the
//! Model Context Protocol (stdio transport).  Designed to run as a child
//! process of OpenCarrier's MCP client.
//!
//! # Configuration (environment variables)
//!
//! | Variable                | Description                |
//! |-------------------------|----------------------------|
//! | `WECHAT_OA_APP_ID`     | 公众号 AppID               |
//! | `WECHAT_OA_APP_SECRET` | 公众号 AppSecret            |
//! | `WECHAT_OA_MCP_LOG`    | Log level (default `warn`)  |

mod wechat;

use std::sync::Arc;

use anyhow::Result;
use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, ServiceExt, transport::stdio as stdio_transport};
use schemars::JsonSchema;
use serde::Deserialize;
use wechat::WeChatClient;

// ================================================================== //
//  Tool parameter structs                                              //
// ================================================================== //

#[derive(Debug, Deserialize, JsonSchema)]
struct UploadMediaParams {
    #[schemars(description = "Media type: image, voice, video, thumb")]
    media_type: String,
    #[schemars(description = "Filename (e.g. cover.jpg)")]
    filename: String,
    #[schemars(description = "Base64-encoded media data")]
    data_base64: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateDraftParams {
    #[schemars(description = "Article title")]
    title: String,
    #[schemars(description = "Article HTML content")]
    content: String,
    #[schemars(description = "Author name")]
    author: Option<String>,
    #[schemars(description = "Original article URL")]
    content_source_url: Option<String>,
    #[schemars(description = "Article digest / summary")]
    digest: Option<String>,
    #[schemars(description = "Cover image media_id (upload_media returns this)")]
    thumb_media_id: Option<String>,
    #[schemars(description = "Show cover in article body (1=yes 0=no, default 1)")]
    need_open_comment: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetDraftParams {
    #[schemars(description = "Draft media_id")]
    media_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListDraftsParams {
    #[schemars(description = "Page offset (0-based, default 0)")]
    offset: Option<i32>,
    #[schemars(description = "Page size (max 20, default 20)")]
    count: Option<i32>,
    #[schemars(description = "Set to 1 to omit article content (saves bandwidth)")]
    no_content: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteDraftParams {
    #[schemars(description = "Draft media_id to delete")]
    media_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PublishDraftParams {
    #[schemars(description = "Draft media_id to publish")]
    media_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetPublishStatusParams {
    #[schemars(description = "publish_id returned by publish_draft")]
    publish_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListMaterialsParams {
    #[schemars(description = "Material type: image, video, voice, news")]
    r#type: String,
    #[schemars(description = "Page offset (0-based, default 0)")]
    offset: Option<i32>,
    #[schemars(description = "Page size (max 20, default 20)")]
    count: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteMaterialParams {
    #[schemars(description = "Material media_id to delete")]
    media_id: String,
}

// ================================================================== //
//  MCP Server                                                         //
// ================================================================== //

#[derive(Clone)]
struct WeChatOaServer {
    client: Arc<WeChatClient>,
}

#[tool_router(server_handler)]
impl WeChatOaServer {
    // ---- Token ----

    #[tool(description = "Get WeChat OA access token (auto-refreshed, cached for ~2 hours)")]
    async fn get_access_token(&self) -> String {
        match self.client.get_token().await {
            Ok(token) => serde_json::json!({ "access_token": token }).to_string(),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ---- Media ----

    #[tool(description = "Upload image/media to WeChat OA permanent material library. Returns media_id and url.")]
    async fn upload_media(
        &self,
        Parameters(params): Parameters<UploadMediaParams>,
    ) -> String {
        let data = match base64::engine::general_purpose::STANDARD.decode(&params.data_base64) {
            Ok(d) => d,
            Err(e) => return format!("{{\"error\": \"invalid base64: {}\"}}", e),
        };
        match self
            .client
            .upload_media(&params.media_type, &params.filename, &data)
            .await
        {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ---- Drafts ----

    #[tool(description = "Create a new draft article. Returns media_id of the draft.")]
    async fn create_draft(
        &self,
        Parameters(params): Parameters<CreateDraftParams>,
    ) -> String {
        let article = serde_json::json!({
            "title": params.title,
            "content": params.content,
            "author": params.author.unwrap_or_default(),
            "content_source_url": params.content_source_url.unwrap_or_default(),
            "digest": params.digest.unwrap_or_default(),
            "thumb_media_id": params.thumb_media_id.unwrap_or_default(),
            "need_open_comment": params.need_open_comment.unwrap_or(1),
            "only_fans_can_comment": 0,
        });
        let body = serde_json::json!({ "articles": [article] });
        match self.client.api_post("/cgi-bin/draft/add", &body).await {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "Get full draft content by media_id")]
    async fn get_draft(
        &self,
        Parameters(params): Parameters<GetDraftParams>,
    ) -> String {
        let body = serde_json::json!({ "media_id": params.media_id });
        match self.client.api_post("/cgi-bin/draft/get", &body).await {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "List drafts in the WeChat OA draft box")]
    async fn list_drafts(
        &self,
        Parameters(params): Parameters<ListDraftsParams>,
    ) -> String {
        let body = serde_json::json!({
            "offset": params.offset.unwrap_or(0),
            "count": params.count.unwrap_or(20),
            "no_content": params.no_content.unwrap_or(0),
        });
        match self.client.api_post("/cgi-bin/draft/batchget", &body).await {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "Delete a draft by media_id")]
    async fn delete_draft(
        &self,
        Parameters(params): Parameters<DeleteDraftParams>,
    ) -> String {
        let body = serde_json::json!({ "media_id": params.media_id });
        match self.client.api_post("/cgi-bin/draft/delete", &body).await {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ---- Publishing ----

    #[tool(description = "Submit a draft for publishing. Returns publish_id for status tracking.")]
    async fn publish_draft(
        &self,
        Parameters(params): Parameters<PublishDraftParams>,
    ) -> String {
        let body = serde_json::json!({ "media_id": params.media_id });
        match self
            .client
            .api_post("/cgi-bin/freepublish/submit", &body)
            .await
        {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "Check the publishing status of a submitted draft")]
    async fn get_publish_status(
        &self,
        Parameters(params): Parameters<GetPublishStatusParams>,
    ) -> String {
        let body = serde_json::json!({ "publish_id": params.publish_id });
        match self.client.api_post("/cgi-bin/freepublish/get", &body).await {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    // ---- Materials ----

    #[tool(description = "List permanent materials in the WeChat OA library")]
    async fn list_materials(
        &self,
        Parameters(params): Parameters<ListMaterialsParams>,
    ) -> String {
        let body = serde_json::json!({
            "type": params.r#type,
            "offset": params.offset.unwrap_or(0),
            "count": params.count.unwrap_or(20),
        });
        match self
            .client
            .api_post("/cgi-bin/material/batchget_material", &body)
            .await
        {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }

    #[tool(description = "Delete a permanent material by media_id")]
    async fn delete_material(
        &self,
        Parameters(params): Parameters<DeleteMaterialParams>,
    ) -> String {
        let body = serde_json::json!({ "media_id": params.media_id });
        match self
            .client
            .api_post("/cgi-bin/material/del_material", &body)
            .await
        {
            Ok(resp) => json_to_string(&resp),
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        }
    }
}

// ================================================================== //
//  Helpers                                                             //
// ================================================================== //

fn json_to_string(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|e| format!("{{\"error\": \"serialize: {}\"}}", e))
}

// ================================================================== //
//  Entry point                                                         //
// ================================================================== //

#[tokio::main]
async fn main() -> Result<()> {
    // Log to stderr — stdout is reserved for the MCP protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("WECHAT_OA_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let app_id = std::env::var("WECHAT_OA_APP_ID")
        .map_err(|_| anyhow::anyhow!("WECHAT_OA_APP_ID env var not set"))?;
    let app_secret = std::env::var("WECHAT_OA_APP_SECRET")
        .map_err(|_| anyhow::anyhow!("WECHAT_OA_APP_SECRET env var not set"))?;

    let client = WeChatClient::new(app_id, app_secret);
    let server = WeChatOaServer {
        client: Arc::new(client),
    };

    tracing::info!("wechat-oa-mcp starting (stdio transport)");
    let service = server.serve(stdio_transport()).await?;
    service.waiting().await?;

    Ok(())
}
