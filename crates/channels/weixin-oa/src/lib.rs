//! WeChat Official Account (公众号/服务号) channel adapter for OpenCarrier.
//!
//! Webhook-based channel that receives WeChat OA messages via HTTP callback,
//! translates them into PluginMessages, and routes them to agents.
//! Replies are sent via the WeChat customer service message API.

pub mod api;
pub mod channel;
pub mod models;
pub mod tools;

pub use channel::{
    build_plugin_message, deliver_content, execute_push, needs_reply, note_inbound_activity,
    opens_cs_window, OaAccountState, SessionWatcher, WeixinOaState, WEIXIN_OA_STATE,
};
pub use models::{parse_xml_message, OaMessage, ProxyMessage, WeixinOaSessionFile};
pub use tools::WeixinOaDraftListTool;
pub use tools::WeixinOaPublishArticleTool;
