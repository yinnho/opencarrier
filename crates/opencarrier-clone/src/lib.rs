//! OpenCarrier Clone Loader — .agx template loading and conversion.
//!
//! Loads .agx clone archives (from openclone-core) and converts them into
//! OpenCarrier AgentManifest + workspace files.

mod loader;
mod converter;
pub mod hub;

pub use loader::{CloneData, load_agx, pack_agx, SkillData, SkillScriptData, AgentData, TemplateManifest};
pub use converter::{convert_to_manifest, install_clone_to_workspace};
