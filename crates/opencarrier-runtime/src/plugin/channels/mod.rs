//! Built-in channel adapters — compiled into the main binary.
//!
//! Channels:
//! - `weixin` — WeChat iLink personal account
//! - `wecom` — WeCom (enterprise WeChat)
//! - `feishu` — Feishu / Lark
//! - `dingtalk` — DingTalk (钉钉)

pub mod dingtalk;
pub mod feishu;
pub mod wecom;
pub mod weixin;
