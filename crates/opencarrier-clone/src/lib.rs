//! OpenCarrier Clone Loader — .agx template loading and conversion.
//!
//! Loads .agx clone archives (from openclone-core) and converts them into
//! OpenCarrier AgentManifest + workspace files.

mod converter;
pub mod hub;
mod loader;

pub use converter::{convert_to_manifest, install_clone_to_workspace};
pub use loader::{
    format_string_array, load_agx, pack_agx, parse_frontmatter, parse_string_array,
    parse_toml_description, AgentData, CloneData, SkillData, SkillScriptData, TemplateManifest,
};
