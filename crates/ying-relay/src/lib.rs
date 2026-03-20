//! ying-relay - Relay WebSocket Client for OpenCarrier
//!
//! 连接到 relay.yinnho.cn，实现：
//! - WebSocket 连接管理
//! - Ed25519 签名认证
//! - 消息加密/解密 (AES-256-GCM)
//! - 心跳保活
//! - 断线重连
//!
//! 与 yingheclient 的 relay-connection.ts 功能对等。

mod auth;
mod client;
mod crypto;
mod protocol;

pub use auth::*;
pub use client::*;
pub use crypto::*;
pub use protocol::*;
