//! WeChat iLink plugin tools — built-in, no FFI.

use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

use crate::plugin::channels::weixin::auth;
use crate::plugin::channels::weixin::token::WEIXIN_STATE;

// ---------------------------------------------------------------------------
// QR Login tool
// ---------------------------------------------------------------------------

/// Tool: Trigger QR code login for a WeChat account.
pub struct WeixinQrLoginTool;

impl ToolProvider for WeixinQrLoginTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "weixin_qr_login".to_string(),
            description: "Trigger WeChat iLink QR code login. Returns a QR code URL for the user to scan with WeChat. After scanning, the bot token is saved automatically.".to_string(),
            parameters_json: r#"{"type":"object","properties":{"tenant_name":{"type":"string","description":"Name for this WeChat account (used as tenant ID)"}},"required":["tenant_name"]}"#.to_string(),
        }
    }

    fn execute(&self, args: &Value, _context: &PluginToolContext) -> Result<String, PluginError> {
        let tenant_name = args["tenant_name"]
            .as_str()
            .unwrap_or("default")
            .to_string();

        let http = reqwest::Client::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        let tenant = tenant_name.clone();
        let result = rt.block_on(async { auth::qr_login(&http, &tenant, None).await });

        result.map_err(PluginError::tool)
    }
}

// ---------------------------------------------------------------------------
// Send Message tool
// ---------------------------------------------------------------------------

/// Tool: Send a message to a WeChat user via iLink.
pub struct WeixinSendMessageTool;

impl ToolProvider for WeixinSendMessageTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "weixin_send_message".to_string(),
            description: "Send a text message to a WeChat user via iLink. Requires an active QR-logged-in session. You can only reply to users who have already sent a message (context_token required).".to_string(),
            parameters_json: r#"{"type":"object","properties":{"tenant_name":{"type":"string","description":"Tenant name (WeChat account)"},"user_id":{"type":"string","description":"iLink user ID to send to"},"text":{"type":"string","description":"Message text"}},"required":["tenant_name","user_id","text"]}"#.to_string(),
        }
    }

    fn execute(&self, args: &Value, _context: &PluginToolContext) -> Result<String, PluginError> {
        let tenant_name = args["tenant_name"]
            .as_str()
            .ok_or_else(|| PluginError::tool("missing tenant_name"))?;
        let user_id = args["user_id"]
            .as_str()
            .ok_or_else(|| PluginError::tool("missing user_id"))?;
        let text = args["text"]
            .as_str()
            .ok_or_else(|| PluginError::tool("missing text"))?;

        let state = WEIXIN_STATE
            .tenants
            .get(tenant_name)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {tenant_name}")))?;

        if state.is_expired() {
            return Err(PluginError::tool("Token expired, please re-scan QR code"));
        }

        let context_token = state
            .get_context_token(user_id)
            .ok_or_else(|| {
                PluginError::tool(format!(
                    "No context_token for user {user_id} — can only reply to received messages"
                ))
            })?;

        let bot_token = state.bot_token.clone();
        let baseurl = state.baseurl.clone();
        let http = state.http.clone();
        let client_id = format!("openclaw-weixin-{}", uuid::Uuid::new_v4().as_simple());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::tool(format!("Runtime error: {e}")))?;

        rt.block_on(async {
            crate::plugin::channels::weixin::api::send_message(
                &http,
                &bot_token,
                &baseurl,
                user_id,
                &context_token,
                &client_id,
                text,
            )
            .await
            .map_err(PluginError::tool)
        })?;

        Ok("Message sent".to_string())
    }
}

// ---------------------------------------------------------------------------
// Status tool
// ---------------------------------------------------------------------------

/// Tool: Show status of all iLink tenants.
pub struct WeixinStatusTool;

impl ToolProvider for WeixinStatusTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "weixin_status".to_string(),
            description: "Show status of all linked WeChat accounts (tenants). Shows which are active, expired, or waiting for QR scan.".to_string(),
            parameters_json: r#"{"type":"object","properties":{}}"#.to_string(),
        }
    }

    fn execute(&self, _args: &Value, _context: &PluginToolContext) -> Result<String, PluginError> {
        let statuses = WEIXIN_STATE.status_list();
        if statuses.is_empty() {
            return Ok("No WeChat accounts linked. Use weixin_qr_login to link one.".to_string());
        }
        Ok(serde_json::to_string_pretty(&statuses).unwrap_or_else(|_| "Status error".to_string()))
    }
}
