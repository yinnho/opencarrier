//! WeCom tool providers — messaging via channel REST API.
//!
//! Document/spreadsheet/contact/todo/meeting/schedule tools are now provided
//! by the MCP module (`crate::mcp`).

use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

use crate::token;

// ---------------------------------------------------------------------------
// Send WeCom Message (App and Kf modes — uses channel-layer REST API)
// ---------------------------------------------------------------------------

pub struct SendMessageTool;

impl ToolProvider for SendMessageTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "send_wecom_message",
            "发送企业微信消息给指定用户（支持企业应用和微信客服模式）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": { "type": "string", "description": "接收人UserID（企业应用为内部UserID，微信客服为external_userid）" },
                    "content": { "type": "string", "description": "消息内容" }
                },
                "required": ["user_id", "content"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let user_id = args["user_id"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing user_id"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing content"))?;

        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        match &tenant.mode {
            token::WecomMode::App { .. } => {
                token::send_app_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::Kf { .. } => {
                token::send_kf_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::SmartBot { .. } => {
                return Err(PluginError::tool(
                    "SmartBot mode replies via response_url, not send_wecom_message",
                ));
            }
        }

        Ok(format!("Message sent to {}", user_id))
    }
}
