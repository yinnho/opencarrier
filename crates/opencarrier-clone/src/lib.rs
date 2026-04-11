//! OpenCarrier Clone Loader — .agx template loading and conversion.
//!
//! Loads .agx clone archives (from openclone-core) and converts them into
//! OpenCarrier AgentManifest + workspace files.

mod loader;
mod converter;
pub mod hub;

pub use loader::{CloneData, load_agx, SkillData, SkillScriptData};
pub use converter::{convert_to_manifest, install_clone_to_workspace};
