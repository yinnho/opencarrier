//! Core kernel for the OpenCarrier Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod brain;
pub mod background;
pub mod capabilities;
pub mod config;
pub mod config_reload;
pub mod cron;
pub mod error;
pub mod event_bus;
pub mod heartbeat;
pub mod kernel;
pub mod metering;
pub mod registry;
pub mod scheduler;
pub mod supervisor;
pub mod wizard;
pub use kernel::OpenCarrierKernel;
