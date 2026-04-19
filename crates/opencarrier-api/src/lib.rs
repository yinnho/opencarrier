//! HTTP/WebSocket API server for the OpenCarrier Agent OS daemon.
//!
//! Exposes agent management, status, and chat via JSON REST endpoints.
//! The kernel runs in-process; the CLI connects over HTTP.

pub mod command_security;
pub mod middleware;
pub mod rate_limiter;
pub mod routes;
pub mod server;
pub mod session_auth;
pub mod stream_chunker;
pub mod stream_dedup;
pub mod types;
pub mod webchat;
pub mod ws;
