//! Built-in channel adapters — compiled into the main binary.
//!
//! Channels:
//! - `weixin` — WeChat iLink personal account
//! - `wecom` — WeCom (enterprise WeChat)
//! - `feishu` — Feishu / Lark

pub mod feishu;
pub mod wecom;
pub mod weixin;
