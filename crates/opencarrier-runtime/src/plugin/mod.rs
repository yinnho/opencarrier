//! Plugin system — dynamically loadable channel adapters and tools.
//!
//! Provides the infrastructure for loading shared library plugins at runtime
//! via `dlopen`. Each plugin can register:
//! - **Channels**: message adapters that bridge external platforms to the kernel
//! - **Tools**: platform API capabilities that agents can call
//!
//! See `docs/PLUGIN-SYSTEM-DESIGN.md` for the full architecture.

pub mod bridge;
pub mod builtin;
pub mod builtin_registry;
pub mod channels;
pub mod instance;
pub mod loader;
pub mod manager;
pub mod tool_dispatch;

pub use builtin::{BuiltinChannel, BuiltinPlugin};
pub use builtin_registry::BuiltinPluginRegistry;
pub use instance::PluginInstance;
pub use loader::LoadedPlugin;
pub use manager::PluginManager;
