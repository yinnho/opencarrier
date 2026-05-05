//! Shared utilities for OpenCarrier MCP servers.
//!
//! Provides cookie-auth macros, JSON helpers, and a generic HTTP API client
//! so individual MCP servers don't duplicate boilerplate.

pub mod api;
pub mod cookie;
pub mod json;
