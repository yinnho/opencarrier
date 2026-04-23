//! Plugin system — dynamically loadable channel adapters and tools.
//!
//! Provides the infrastructure for loading shared library plugins at runtime
//! via `dlopen`. Each plugin can register:
//! - **Channels**: message adapters that bridge external platforms to the kernel
//! - **Tools**: platform API capabilities that agents can call
//!
//! See `docs/PLUGIN-SYSTEM-DESIGN.md` for the full architecture.

pub mod loader;
pub mod manager;
pub mod bridge;
pub mod tool_dispatch;

pub use loader::LoadedPlugin;
pub use manager::PluginManager;
