//! Ratatui TUI modules for OpenCarrier.
//!
//! Active modules: `chat_runner` (interactive chat), `event` (event loop),
//! `theme` (shared styles). The dashboard was removed in favor of the web UI.

pub mod chat_runner;
pub mod event;
pub mod screens;
pub mod theme;
