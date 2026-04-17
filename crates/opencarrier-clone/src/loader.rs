//! .agx archive loader — decompresses tar.gz and parses clone data.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Parsed template.json from the .agx archive.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TemplateManifest {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub knowledge_version: u32,
}

/// A parsed skill from the .agx archive.
#[derive(Debug, Clone)]
pub struct SkillData {
    pub name: String,
    pub when_to_use: String,
    pub allowed_tools: Vec<String>,
    pub prompt: String,
    pub scripts: Vec<SkillScriptData>,
}

/// A parsed skill script (HTTP API definition).
#[derive(Debug, Clone)]
pub struct SkillScriptData {
    pub name: String,
    pub description: String,
    pub toml_content: String,
}

/// A parsed sub-agent from the .agx archive.
#[derive(Debug, Clone)]
pub struct AgentData {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: String,
    pub color: Option<String>,
    pub prompt: String,
}

/// The fully parsed .agx clone data.
#[derive(Debug, Clone)]
pub struct CloneData {
    /// Template manifest (from template.json).
    pub manifest: Option<TemplateManifest>,
    /// Clone name (from profile.md frontmatter or filename).
    pub name: String,
    /// Clone description.
    pub description: String,
    /// SOUL.md content — personality definition.
    pub soul: String,
    /// system_prompt.md content — behavioral instructions.
    pub system_prompt: String,
    /// MEMORY.md content — knowledge index.
    pub memory_index: String,
    /// Knowledge files: filename → content.
    pub knowledge: HashMap<String, String>,
    /// Parsed skills.
    pub skills: Vec<SkillData>,
    /// Raw profile.md content.
    pub profile: String,
    /// Security warnings found during loading.
    pub security_warnings: Vec<String>,
    /// Sub-agents: agents/*.md parsed.
    pub agents: Vec<AgentData>,
    /// EVOLUTION.md content.
    pub evolution: String,
    /// Style files: filename → content.
    pub style: HashMap<String, String>,
}

/// Load a .agx file (tar.gz) and parse it into CloneData.
pub fn load_agx(path: &Path) -> Result<CloneData> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open .agx file: {}", path.display()))?;

    let mut archive = tar::Archive::new(GzDecoder::new(file));

    // Collect all files into memory
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().to_string();
        // Skip directories
        if name.ends_with('/') {
            continue;
        }
        // Normalize: strip leading "./"
        let name = name.strip_prefix("./").unwrap_or(&name).to_string();
        // Skip macOS Apple Double files (._*)
        if Path::new(&name).file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("._"))
            .unwrap_or(false)
        {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        files.insert(name, buf);
    }

    info!("Loaded .agx archive: {} files", files.len());

    // Parse template.json
    let manifest = files.get("template.json")
        .and_then(|bytes| String::from_utf8_lossy(bytes).into_owned().into())
        .and_then(|s| serde_json::from_str::<TemplateManifest>(&s).ok());

    // Parse profile.md → extract name and description
    let profile = get_file_text(&files, "profile.md");
    let (name, description) = parse_profile(&profile, path);

    // Read core files
    let soul = get_file_text(&files, "SOUL.md");
    let system_prompt = get_file_text(&files, "system_prompt.md");
    let memory_index = get_file_text(&files, "MEMORY.md");

    // Parse knowledge files
    let mut knowledge = HashMap::new();
    for (name, bytes) in &files {
        if name.starts_with("knowledge/") && name.ends_with(".md") {
            let content = String::from_utf8_lossy(bytes).to_string();
            let filename = name.strip_prefix("knowledge/").unwrap_or(name);
            knowledge.insert(filename.to_string(), content);
        }
    }

    // Parse skills
    let skills = parse_skills(&files);

    // Parse agents/*.md
    let mut agents = Vec::new();
    for (name, bytes) in &files {
        if name.starts_with("agents/") && name.ends_with(".md") {
            let content = String::from_utf8_lossy(bytes).to_string();
            if let Some(agent) = parse_agent_file(name, &content) {
                agents.push(agent);
            }
        }
    }

    // Read EVOLUTION.md
    let evolution = get_file_text(&files, "EVOLUTION.md");

    // Parse style/*.md
    let mut style = HashMap::new();
    for (name, bytes) in &files {
        if name.starts_with("style/") && name.ends_with(".md") {
            let content = String::from_utf8_lossy(bytes).to_string();
            let filename = name.strip_prefix("style/").unwrap_or(name);
            style.insert(filename.to_string(), content);
        }
    }

    // Security scan
    let mut security_warnings = Vec::new();
    scan_security(&soul, &system_prompt, &knowledge, &skills, &mut security_warnings);

    debug!(
        "Parsed clone '{}': soul={} bytes, system_prompt={} bytes, knowledge={} files, skills={}, agents={}, style={}, evolution={} bytes, memory_index={} bytes",
        name, soul.len(), system_prompt.len(), knowledge.len(), skills.len(), agents.len(), style.len(), evolution.len(), memory_index.len()
    );

    Ok(CloneData {
        manifest,
        name,
        description,
        soul,
        system_prompt,
        memory_index,
        knowledge,
        skills,
        profile,
        security_warnings,
        agents,
        evolution,
        style,
    })
}

/// Get a text file from the archive, or empty string.
fn get_file_text(files: &HashMap<String, Vec<u8>>, name: &str) -> String {
    files.get(name)
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
        .unwrap_or_default()
}

/// Parse profile.md frontmatter to extract name and description.
fn parse_profile(profile: &str, agx_path: &Path) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();

    // Parse YAML frontmatter
    if profile.starts_with("---") {
        if let Some(end) = profile[3..].find("---") {
            let frontmatter = &profile[3..3 + end];
            for line in frontmatter.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("name:") {
                    name = val.trim().trim_matches('"').to_string();
                } else if let Some(val) = line.strip_prefix("description:") {
                    description = val.trim().trim_matches('"').to_string();
                }
            }
        }
    }

    // Fallback name from filename
    if name.is_empty() {
        name = agx_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown-clone")
            .to_string();
    }

    (name, description)
}

/// Parse all skills from the archive.
fn parse_skills(files: &HashMap<String, Vec<u8>>) -> Vec<SkillData> {
    let mut skills = Vec::new();

    // Collect skill file paths
    let mut skill_files: Vec<String> = files.keys()
        .filter(|n| n.starts_with("skills/") && n.ends_with(".md"))
        .cloned()
        .collect();

    // Also handle directory-based skills: skills/<name>/SKILL.md
    let dir_skills: Vec<String> = files.keys()
        .filter(|n| {
            let parts: Vec<&str> = n.split('/').collect();
            parts.len() == 3 && parts[0] == "skills" && parts[2] == "SKILL.md"
        })
        .cloned()
        .collect();

    skill_files.extend(dir_skills);

    for skill_path in &skill_files {
        let content = match files.get(skill_path) {
            Some(bytes) => String::from_utf8_lossy(bytes).to_string(),
            None => continue,
        };

        let (frontmatter, body) = parse_frontmatter(&content);
        let name = frontmatter.get("name")
            .cloned()
            .unwrap_or_else(|| {
                skill_path.split('/').nth(1).unwrap_or("unknown").to_string()
            });
        let when_to_use = frontmatter.get("when_to_use")
            .cloned()
            .unwrap_or_default();
        let allowed_tools = frontmatter.get("allowed_tools")
            .map(|s| parse_string_array(s))
            .unwrap_or_default();

        // Find scripts for directory-based skills
        let skill_dir = format!("skills/{}/", name);
        let scripts = files.keys()
            .filter(|n| n.starts_with(&skill_dir) && n.ends_with(".toml"))
            .filter_map(|script_path| {
                let toml_content = String::from_utf8_lossy(files.get(script_path)?).to_string();
                let script_name = script_path.split('/').last()?
                    .strip_suffix(".toml")?
                    .to_string();
                let desc = parse_toml_description(&toml_content);
                Some(SkillScriptData {
                    name: script_name,
                    description: desc,
                    toml_content,
                })
            })
            .collect();

        skills.push(SkillData {
            name,
            when_to_use,
            allowed_tools,
            prompt: body.trim().to_string(),
            scripts,
        });
    }

    skills
}

/// Parse an agents/*.md file from the archive.
fn parse_agent_file(path: &str, content: &str) -> Option<AgentData> {
    let (frontmatter, body) = parse_frontmatter(content);
    let name = frontmatter.get("name")
        .cloned()
        .unwrap_or_else(|| {
            path.strip_prefix("agents/")
                .unwrap_or(path)
                .strip_suffix(".md")
                .unwrap_or("unknown")
                .to_string()
        });
    let description = frontmatter.get("description")
        .cloned()
        .unwrap_or_default();
    let tools = frontmatter.get("tools")
        .map(|s| parse_string_array(s))
        .unwrap_or_default();
    let model = frontmatter.get("model")
        .cloned()
        .unwrap_or_else(|| "sonnet".to_string());
    let color = frontmatter.get("color")
        .cloned();
    let prompt = body.trim().to_string();

    Some(AgentData {
        name,
        description,
        tools,
        model,
        color,
        prompt,
    })
}

/// Pack CloneData into .agx bytes (tar.gz).
pub fn pack_agx(data: &CloneData) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let enc = GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);

        // template.json
        if let Some(ref manifest) = data.manifest {
            let json = serde_json::to_string_pretty(manifest)
                .context("Failed to serialize template.json")?;
            append_file(&mut tar, "template.json", json.as_bytes())?;
        }

        // profile.md
        if !data.profile.is_empty() {
            append_file(&mut tar, "profile.md", data.profile.as_bytes())?;
        }

        // SOUL.md
        if !data.soul.is_empty() {
            append_file(&mut tar, "SOUL.md", data.soul.as_bytes())?;
        }

        // system_prompt.md
        if !data.system_prompt.is_empty() {
            append_file(&mut tar, "system_prompt.md", data.system_prompt.as_bytes())?;
        }

        // MEMORY.md
        if !data.memory_index.is_empty() {
            append_file(&mut tar, "MEMORY.md", data.memory_index.as_bytes())?;
        }

        // EVOLUTION.md
        if !data.evolution.is_empty() {
            append_file(&mut tar, "EVOLUTION.md", data.evolution.as_bytes())?;
        }

        // knowledge/*.md
        for (name, content) in &data.knowledge {
            let path = format!("knowledge/{}", name);
            append_file(&mut tar, &path, content.as_bytes())?;
        }

        // skills/*.md (with scripts)
        for skill in &data.skills {
            let content = format_skill_md(skill);
            let path = format!("skills/{}.md", skill.name);
            append_file(&mut tar, &path, content.as_bytes())?;

            // Write scripts if any
            for script in &skill.scripts {
                let script_path = format!("skills/{}/scripts/{}.toml", skill.name, script.name);
                append_file(&mut tar, &script_path, script.toml_content.as_bytes())?;
            }
        }

        // agents/*.md
        for agent in &data.agents {
            let content = format_agent_md(agent);
            let path = format!("agents/{}.md", agent.name);
            append_file(&mut tar, &path, content.as_bytes())?;
        }

        // style/*.md
        for (name, content) in &data.style {
            let path = format!("style/{}", name);
            append_file(&mut tar, &path, content.as_bytes())?;
        }

        tar.into_inner()
            .context("Failed to finalize tar archive")?
            .finish()
            .context("Failed to finalize gzip")?;
    }
    Ok(buf)
}

fn append_file<W: std::io::Write>(tar: &mut tar::Builder<W>, path: &str, data: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, path, data)
        .with_context(|| format!("Failed to add {} to archive", path))
}

fn format_skill_md(skill: &SkillData) -> String {
    let mut buf = String::new();
    buf.push_str("---\n");
    buf.push_str(&format!("name: {}\n", skill.name));
    buf.push_str(&format!("when_to_use: {}\n", skill.when_to_use));
    if !skill.allowed_tools.is_empty() {
        buf.push_str(&format!("allowed_tools: {}\n", format_string_array(&skill.allowed_tools)));
    }
    buf.push_str("---\n\n");
    buf.push_str(&skill.prompt);
    buf
}

fn format_agent_md(agent: &AgentData) -> String {
    let mut buf = String::new();
    buf.push_str("---\n");
    buf.push_str(&format!("name: {}\n", agent.name));
    buf.push_str(&format!("description: {}\n", agent.description));
    if !agent.tools.is_empty() {
        buf.push_str(&format!("tools: {}\n", format_string_array(&agent.tools)));
    }
    if !agent.model.is_empty() {
        buf.push_str(&format!("model: {}\n", agent.model));
    }
    if let Some(ref color) = agent.color {
        buf.push_str(&format!("color: {}\n", color));
    }
    buf.push_str("---\n\n");
    buf.push_str(&agent.prompt);
    buf
}

/// Format a string slice as `["a", "b"]` — safe for YAML frontmatter.
pub fn format_string_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{}\"", s.replace('"', "\\\""))).collect();
    format!("[{}]", quoted.join(", "))
}

/// Parse YAML frontmatter from markdown content.
pub fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    let mut map = HashMap::new();
    if !content.starts_with("---") {
        return (map, content.to_string());
    }

    let rest = &content[3..];
    let Some(end) = rest.find("---") else {
        return (map, content.to_string());
    };

    let frontmatter = &rest[..end];
    let body = &rest[end + 3..];

    // Simple key: value parsing (handles basic YAML)
    let mut current_key = String::new();
    let mut in_array = false;
    let mut array_val = String::new();

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if in_array {
            if trimmed.starts_with('-') || trimmed.starts_with('"') || trimmed.starts_with('[') {
                array_val.push_str(trimmed);
                array_val.push(' ');
            }
            if trimmed.ends_with(']') || (!trimmed.starts_with('-') && !trimmed.starts_with('"') && !trimmed.starts_with('[') && !trimmed.starts_with(' ')) {
                map.insert(current_key.clone(), array_val.trim().to_string());
                in_array = false;
            }
            continue;
        }

        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim().to_string();
            let val = trimmed[colon_pos + 1..].trim().to_string();

            if val.is_empty() {
                // Might be an array on next lines
                current_key = key;
                in_array = true;
                array_val = String::new();
            } else {
                map.insert(key, val.trim_matches('"').to_string());
            }
        }
    }

    (map, body.to_string())
}

/// Parse a string like `["tool1", "tool2"]` or `["tool1","tool2"]` into a Vec.
pub fn parse_string_array(s: &str) -> Vec<String> {
    let s = s.trim();
    if !s.starts_with('[') {
        return vec![s.trim_matches('"').to_string()];
    }

    s.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Extract description from a TOML script file.
pub fn parse_toml_description(content: &str) -> String {
    for line in content.lines() {
        if let Some(val) = line.trim().strip_prefix("description") {
            if let Some(val) = val.trim_start_matches('=').trim().strip_prefix('"') {
                if let Some(val) = val.strip_suffix('"') {
                    return val.to_string();
                }
            }
        }
    }
    String::new()
}

/// Basic security scan for loaded clone data.
fn scan_security(
    soul: &str,
    system_prompt: &str,
    knowledge: &HashMap<String, String>,
    skills: &[SkillData],
    warnings: &mut Vec<String>,
) {
    let injection_keywords = [
        "ignore previous instructions",
        "ignore all previous",
        "jailbreak",
        "you are now",
        "new instructions:",
        "system override",
    ];

    for keyword in &injection_keywords {
        let lower_prompt = system_prompt.to_lowercase();
        if lower_prompt.contains(keyword) {
            warnings.push(format!(
                "System prompt contains potential injection keyword: '{}'",
                keyword
            ));
        }
    }

    // File size checks
    if system_prompt.len() > 1_000_000 {
        warnings.push(format!("system_prompt.md is very large: {} bytes", system_prompt.len()));
    }
    if soul.len() > 500_000 {
        warnings.push(format!("SOUL.md is very large: {} bytes", soul.len()));
    }
    for (name, content) in knowledge {
        if content.len() > 1_000_000 {
            warnings.push(format!("knowledge/{} is very large: {} bytes", name, content.len()));
        }
    }
    for skill in skills {
        for script in &skill.scripts {
            if script.toml_content.contains("http://") && !script.toml_content.contains("localhost") {
                warnings.push(format!(
                    "Skill '{}' script '{}' uses non-HTTPS URL",
                    skill.name, script.name
                ));
            }
        }
    }

    if !warnings.is_empty() {
        warn!("Security scan found {} warnings", warnings.len());
        for w in &*warnings {
            warn!("  - {}", w);
        }
    }
}
