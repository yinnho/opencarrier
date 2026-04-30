//! MCP tool integration — generic ToolProvider that dispatches to WeCom MCP servers.

pub mod auth;
pub mod client;
pub mod config;
pub mod schema;

use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

use crate::TOKEN_MANAGER;
use schema::ToolSpec;

/// Maximum result size (60KB, leaving 4KB margin for the 64KB FFI buffer).
const MAX_RESULT_BYTES: usize = 60_000;

// ---------------------------------------------------------------------------
// McpToolProvider
// ---------------------------------------------------------------------------

/// Generic MCP tool provider — one instance per tool.
pub struct McpToolProvider {
    name: String,
    category: String,
    description: String,
    schema: Value,
}

impl McpToolProvider {
    fn from_spec(spec: &ToolSpec) -> Self {
        Self {
            name: spec.name.to_string(),
            category: spec.category.to_string(),
            description: spec.description.to_string(),
            schema: spec.schema.clone(),
        }
    }
}

impl ToolProvider for McpToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(&self.name, &self.description, self.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| {
                PluginError::tool(format!(
                    "Unknown tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    TOKEN_MANAGER.tenant_names().join(", ")
                ))
            })?;

        let (bot_id, bot_secret) = tenant
            .mcp_credentials()
            .ok_or_else(|| PluginError::tool("This tenant has no MCP bot credentials configured. Add mcp_bot_id and mcp_bot_secret to tenant config."))?;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        let url = {
            let http = tenant.http.clone();
            let tenant_name = ctx.tenant_id.clone();
            let category = self.category.clone();
            let bot_id = bot_id.to_string();
            let bot_secret = bot_secret.to_string();
            config::get_category_url(&tenant_name, &category, &bot_id, &bot_secret, &http)
        };

        let url = url.map_err(PluginError::tool)?;

        let result = rt.block_on(async {
            let http = tenant.http.clone();
            let tool_name = self.name.clone();
            let args = args.clone();
            client::call_tool(&http, &url, &tool_name, &args, None).await
        });

        match result {
            Ok(text) => Ok(truncate_result(text)),
            Err(e) if e == "MCP_AUTH_EXPIRED" => {
                // Invalidate cache and retry once
                config::invalidate_cache(&ctx.tenant_id);

                let url = {
                    let http = tenant.http.clone();
                    let tenant_name = ctx.tenant_id.clone();
                    let category = self.category.clone();
                    let bot_id = bot_id.to_string();
                    let bot_secret = bot_secret.to_string();
                    config::get_category_url(
                        &tenant_name,
                        &category,
                        &bot_id,
                        &bot_secret,
                        &http,
                    )
                };

                let url = url.map_err(PluginError::tool)?;

                let retry = rt.block_on(async {
                    let http = tenant.http.clone();
                    let tool_name = self.name.clone();
                    let args = args.clone();
                    client::call_tool(&http, &url, &tool_name, &args, None).await
                });

                retry.map(truncate_result).map_err(PluginError::tool)
            }
            Err(e) => Err(PluginError::tool(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build all MCP tool providers. Called from Plugin::tools().
pub fn build_mcp_tools() -> Vec<Box<dyn ToolProvider>> {
    schema::all_tools()
        .into_iter()
        .map(|spec| Box::new(McpToolProvider::from_spec(&spec)) as Box<dyn ToolProvider>)
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate_result(text: String) -> String {
    if text.len() > MAX_RESULT_BYTES {
        let truncated = &text[..MAX_RESULT_BYTES];
        // Try to find a valid UTF-8 boundary
        let boundary = truncated
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(MAX_RESULT_BYTES);
        format!("{}...\n(truncated, full result is {} bytes)", &text[..boundary], text.len())
    } else {
        text
    }
}
