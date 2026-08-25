//! Filesystem tools: file_read, file_write, file_list, file_convert.

use crate::tool_context::ToolContext;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use types::error::{CarrierError, CarrierResult};
use types::tool::ToolDefinition;

// ---------------------------------------------------------------------------
// Module struct
// ---------------------------------------------------------------------------

pub struct FilesystemTools;

// ---------------------------------------------------------------------------
// ToolModule implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl super::ToolModule for FilesystemTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "file_read".to_string(),
                description: "Read the contents of a file. Paths are relative to the agent workspace.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "The file path to read" }
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "file_write".to_string(),
                description: "Write content to a file. Use 'output/' prefix for user-specific task outputs (articles, reports, drafts, generated content). Use 'memory/' prefix for user-specific private notes. Paths are sandboxed per-user automatically. On success the result includes view_url — paste that link so the user can open the file in a browser.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "The file path to write to" },
                        "content": { "type": "string", "description": "The content to write" }
                    },
                    "required": ["path", "content"]
                }),
            },
            ToolDefinition {
                name: "file_list".to_string(),
                description: "List files in a directory. Paths are relative to the agent workspace.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "The directory path to list" }
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "file_convert".to_string(),
                description: "Convert a document between formats using Pandoc. Supported formats: markdown, html, docx, pdf, rst, latex, etc.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input_path": { "type": "string", "description": "Path to the input file" },
                        "output_format": { "type": "string", "description": "Target format (e.g. 'pdf', 'docx', 'html')" },
                        "output_path": { "type": "string", "description": "Optional output path. Auto-generated if not provided." }
                    },
                    "required": ["input_path", "output_format"]
                }),
            },
        ]
    }

    async fn execute(
        &self,
        name: &str,
        input: &Value,
        ctx: &ToolContext<'_>,
    ) -> Option<CarrierResult<String>> {
        match name {
            "file_read" => Some(tool_file_read(input, ctx).await),
            "file_write" => Some(tool_file_write(input, ctx).await),
            "file_list" => Some(tool_file_list(input, ctx).await),
            "file_convert" => Some(tool_file_convert(input, ctx).await),
            _ => None,
        }
    }

    fn permission_level(&self, tool_name: &str) -> types::tool::PermissionLevel {
        match tool_name {
            "file_read" | "file_list" | "file_convert" => types::tool::PermissionLevel::ReadOnly,
            "file_write" => types::tool::PermissionLevel::Write,
            _ => types::tool::PermissionLevel::Dangerous,
        }
    }
}

// ---------------------------------------------------------------------------
// Private tool implementations
// ---------------------------------------------------------------------------

/// Detect common binary file types from magic bytes.
/// Returns a human-readable kind (e.g. "PNG 图片") so we can tell the LLM
/// to use image_analyze instead of file_read.
fn detect_binary_kind(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("PNG 图片")
    } else if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("JPEG 图片")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("GIF 图片")
    } else if header.starts_with(b"RIFF") && header.len() > 11 && &header[8..12] == b"WEBP" {
        Some("WebP 图片")
    } else if header.len() > 4 && &header[4..8] == b"ftyp" {
        Some("视频文件")
    } else if header.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        Some("PDF 文档")
    } else if header.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        Some("ZIP 压缩包")
    } else {
        None
    }
}

/// Binary document formats file_read can't read as text, but markitdown can
/// extract. Images/video are intentionally NOT here - those go to image_analyze.
const DOCUMENT_EXTS: &[&str] = &[
    "pdf", "docx", "doc", "xlsx", "xls", "pptx", "ppt", "odt", "ods", "odp", "rtf", "epub",
];

/// Return the lowercased extension if `path` is a document format markitdown
/// handles, else None.
fn document_extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    if DOCUMENT_EXTS.contains(&ext.as_str()) {
        Some(ext)
    } else {
        None
    }
}

/// Extract text from a binary document (pdf/docx/xlsx/pptx/...) by shelling out
/// to `markitdown`, which converts many formats to markdown for LLM consumption.
/// Mirrors the `file_convert` (pandoc) shell-out pattern. Returns an error
/// (never falls back to a raw text read) when markitdown is absent or fails,
/// since the file is binary and unreadable as text.
async fn extract_document_with_markitdown(path: &Path, raw_path: &str) -> CarrierResult<String> {
    // Guard against huge files - markitdown + its parsers can be slow/memory-heavy.
    let size = tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if size > 50 * 1024 * 1024 {
        return Err(CarrierError::InvalidInput(format!(
            "文件 '{raw_path}' 太大（{size} bytes，上限 50MB），无法提取文本。"
        )));
    }

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new("markitdown")
            .arg(path)
            .output(),
    )
    .await;

    let output = match out {
        Ok(Ok(o)) => o,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CarrierError::Config(
                "未安装 markitdown，无法读取文档格式（pdf/docx/xlsx/pptx 等）。\
                 请管理员安装：pip install 'markitdown[all]'。"
                    .to_string(),
            ))
        }
        Ok(Err(e)) => return Err(CarrierError::Internal(format!("运行 markitdown 失败：{e}"))),
        Err(_) => {
            return Err(CarrierError::Internal(format!(
                "markitdown 提取 '{raw_path}' 超时（120s）。文件可能过大或格式异常。"
            )))
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CarrierError::Internal(format!(
            "markitdown 提取 '{raw_path}' 失败：{}",
            stderr.trim()
        )));
    }

    let md = String::from_utf8_lossy(&output.stdout).into_owned();
    if md.trim().is_empty() {
        return Err(CarrierError::InvalidInput(format!(
            "markitdown 从 '{raw_path}' 提取的内容为空（可能是扫描件/图片型 PDF 或受保护文档）。\
             图片型内容可用 image_analyze。"
        )));
    }
    // Truncate very large extractions (char-safe) so we don't blow the context.
    if md.len() > 200_000 {
        let head: String = md.chars().take(50_000).collect();
        Ok(format!(
            "{head}\n\n…（内容过长，已截断显示前 50000 字符，原文共 {n} 字节）",
            n = md.len()
        ))
    } else {
        Ok(md)
    }
}

/// Resolve output/memory (and catch-all) paths to the top-level senders directory.
///
/// Returns `None` if the path is a workspace-internal path (knowledge/, flows/, etc.)
/// that should be handled by the sandbox instead.
pub(crate) fn resolve_user_data_path(
    raw_path: &str,
    home_dir: &Path,
    sender_id: &str,
    owner_id: Option<&str>,
    agent_name: &str,
) -> Option<CarrierResult<PathBuf>> {
    // Absolute paths — delegate to the workspace sandbox, which strips the
    // workspace_root prefix and canonicalizes.  We MUST NOT strip the leading
    // slash ourselves (that would turn "/home/…/output/file.md" into
    // "home/…/output/file.md" and join it under the sender's output dir,
    // creating a malformed nested path).
    let normalized = raw_path.replace('\\', "/");
    if normalized.starts_with('/') {
        return None;
    }
    let rel = normalized.trim_start_matches('/');

    // Determine subdirectory and rest-of-path from the user's input
    let (subdir, rest) = if rel.starts_with("output/") || rel == "output" {
        let rest = rel.strip_prefix("output").unwrap_or("");
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        ("output", rest)
    } else if rel.starts_with("memory/") || rel == "memory" {
        let rest = rel.strip_prefix("memory").unwrap_or("");
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        ("memory", rest)
    } else if rel.starts_with("input/") || rel == "input" {
        // input/ holds files the user sent to the agent (saved by the channel
        // bridge into senders/{sender}/input/). Route there so file_read /
        // file_list / file_convert can read received attachments. Writes to
        // input/ are blocked in tool_file_write to protect received files.
        let rest = rel.strip_prefix("input").unwrap_or("");
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        ("input", rest)
    } else if crate::workspace_sandbox::is_internal_path(rel) {
        // Internal paths go through sandbox
        return None;
    } else {
        // Catch-all: non-internal paths go to output/
        ("output", rel)
    };

    // Validate no path traversal
    if let Err(e) = super::validate_path(rel) {
        return Some(Err(e));
    }

    let oid = owner_id.unwrap_or(sender_id);
    let base = types::config::sender_data_dir(home_dir, oid, agent_name, Some(sender_id));
    let target = if rest.is_empty() {
        base.join(subdir)
    } else {
        base.join(subdir).join(rest)
    };

    Some(Ok(target))
}

/// Actionable error when file_read is asked to read a directory. A directory
/// path is the #1 trigger of file_read tool loops: the agent retries on
/// *different* dir paths, each producing a cryptic OS error and evading the
/// exact-match loop guard. Steer it to file_list — mirroring file_list's
/// reverse hint when it is given a file.
fn directory_read_hint(raw_path: &str) -> String {
    format!(
        "路径 '{raw_path}' 是一个目录，不是文件。file_read 只能读取文件内容，不能读目录。\n\
         修正方法：\n\
         - 想列出该目录下的文件 → 用 file_list(path=\"{raw_path}\")\n\
         - 想读取目录里的某个文件 → 用 file_read 并补上文件名（例如 {raw_path}/正文.md）"
    )
}

async fn tool_file_read(input: &Value, ctx: &ToolContext<'_>) -> CarrierResult<String> {
    let raw_path = input["path"].as_str().ok_or(CarrierError::InvalidInput(
        "Missing 'path' parameter".to_string(),
    ))?;

    let resolved =
        if let (Some(hd), Some(sid), Some(an)) = (ctx.home_dir, ctx.sender_id, ctx.agent_name) {
            match resolve_user_data_path(raw_path, hd, sid, ctx.owner_id, an) {
                Some(Ok(path)) => path,
                Some(Err(e)) => return Err(e),
                None => {
                    // Internal path — go through sandbox
                    super::resolve_file_path_for_read(
                        raw_path,
                        ctx.workspace_root,
                        ctx.sender_id,
                        ctx.agent_name,
                    )?
                }
            }
        } else {
            super::resolve_file_path_for_read(
                raw_path,
                ctx.workspace_root,
                ctx.sender_id,
                ctx.agent_name,
            )?
        };

    tracing::info!(raw_path, resolved = %resolved.display(), "file_read resolved path");

    // Existence gate FIRST — before the document-format dispatch below. Two
    // ordering/classification bugs fixed (2026-08-25 review):
    // 1. A probe of a nonexistent .docx/.pdf path used to fall into the
    //    markitdown path and return an error — resurrecting, on every
    //    document-format probe, the error-tracker pollution the
    //    ENOENT-as-answer fix exists to eliminate.
    // 2. metadata errors other than NotFound (permission, ENOTDIR, transient
    //    I/O) were all answered "不存在" — actively wrong: it told the model a
    //    permission-walled file was absent and invited file_write over it.
    let metadata = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // ENOENT on a deliberate existence probe is an ANSWER ("does not
            // exist"), not a failure. Returning Err poisoned the error tracker
            // and burned no-progress idle streaks: the "file_read BEFORE
            // file_write" protocol commands pre-write probes, so a model
            // following instructions was killed by the stuck governor mid-pivot
            // (2026-08-22 86bus: article-brief fetched its URL, probed
            // 素材.md/状态.md ENOENT ×4, died one step before file_write).
            // Even after making the error message explicit ("don't retry, write
            // now"), the model still probed — the error itself (is_error=true)
            // triggered "try different path" behavior regardless of content.
            // Return Ok instead: probe succeeds, answer is "doesn't exist",
            // counts as progress, model moves on to file_write.
            return Ok(format!(
                "文件 '{raw_path}'{}，请直接用 file_write 写入内容。",
                types::tool::FILE_READ_ENOENT_MARKER
            ));
        }
        Err(e) => {
            return Err(CarrierError::InvalidInput(format!(
                "无法访问文件 '{raw_path}'：{e}。这不是\"不存在\"——路径被权限或类型错误挡住了，请修正路径或检查权限。"
            )));
        }
    };

    // Binary document formats (pdf/docx/xlsx/pptx/odt/...) - extract text via
    // markitdown so the agent can read user-sent documents, not just plain text.
    // Images/video are not documents and fall through to the binary-refuse path
    // (use image_analyze). markitdown not installed => clear error (no fallback
    // to a raw text read, since the file is binary).
    if document_extension(&resolved).is_some() {
        return extract_document_with_markitdown(&resolved, raw_path).await;
    }

    // Friendly error: detect binary files (images, etc.) before reading.
    // file_read only handles text; binary files should use image_analyze etc.
    if metadata.is_file() {
        // Check magic bytes to detect common binary formats
        if let Ok(header) = tokio::fs::read(&resolved).await {
            let kind = detect_binary_kind(&header);
            if let Some(kind) = kind {
                return Err(CarrierError::InvalidInput(format!(
                    "文件 '{raw_path}' 是二进制文件（{kind}），file_read 只能读取文本文件。\
                     如果是图片，请用 image_analyze 工具分析；如果是其他二进制文件，\
                     请直接使用它的路径/URL，不需要读取内容。"
                )));
            }
        }
    } else if metadata.is_dir() {
        // Reading a directory is the #1 file_read loop trigger (see
        // directory_read_hint): without an actionable hint the agent retries
        // on different dir paths and evades the exact-match loop guard.
        return Err(CarrierError::InvalidInput(directory_read_hint(raw_path)));
    }

    tokio::fs::read_to_string(&resolved).await.map_err(|e| {
        // Friendly message for UTF-8 decode failures on text files
        if e.to_string().contains("stream did not contain valid UTF-8")
            || e.to_string().contains("invalid utf-8")
        {
            CarrierError::InvalidInput(format!(
                "文件 '{raw_path}' 包含非 UTF-8 内容（可能是二进制文件）。\
                     file_read 只能读文本。如果是图片，请用 image_analyze；\
                     如果是文档，请确认文件格式或使用对应的解析工具。"
            ))
        } else {
            CarrierError::Internal(format!("Failed to read file: {e}"))
        }
    })
}

async fn tool_file_write(input: &Value, ctx: &ToolContext<'_>) -> CarrierResult<String> {
    let raw_path = input["path"].as_str().ok_or(CarrierError::InvalidInput(
        "Missing 'path' parameter".to_string(),
    ))?;

    // Reject replacement characters (U+FFFD) in paths: a corrupted filename
    // (e.g. LLM emitting broken UTF-8 for a Chinese name) is un-typeable by
    // the model afterwards, so any follow-up read/patch/delete of the file
    // fails and loops (2026-08-21 86bus incident).
    if raw_path.contains('\u{FFFD}') {
        return Err(CarrierError::InvalidInput(format!(
            "路径 '{raw_path}' 含损坏字符（U+FFFD），无法写入。请换一个干净的文件名（中文名或 ASCII 名，例如 output/material.md）重试。"
        )));
    }

    // input/ is the user's inbox (attachments they sent, saved by the channel
    // bridge). It's read-only from the agent's side - block writes here so a
    // file_write can't overwrite a received file. Direct output to output/.
    let normalized = raw_path.replace('\\', "/");
    if normalized == "input" || normalized.starts_with("input/") {
        return Err(CarrierError::InvalidInput(
            "input/ 是用户发来的文件收件箱（只读），请改用 output/ 前缀写文件。".to_string(),
        ));
    }

    let resolved =
        if let (Some(hd), Some(sid), Some(an)) = (ctx.home_dir, ctx.sender_id, ctx.agent_name) {
            match resolve_user_data_path(raw_path, hd, sid, ctx.owner_id, an) {
                Some(Ok(path)) => path,
                Some(Err(e)) => return Err(e),
                None => {
                    // Internal path — go through sandbox
                    if let Some(root) = ctx.workspace_root {
                        crate::workspace_sandbox::resolve_sandbox_path_for_write(
                            raw_path,
                            root,
                            ctx.sender_id,
                            ctx.agent_name,
                            ctx.is_clone_admin,
                        )?
                    } else {
                        let _ = super::validate_path(raw_path)?;
                        PathBuf::from(raw_path)
                    }
                }
            }
        } else if let Some(root) = ctx.workspace_root {
            crate::workspace_sandbox::resolve_sandbox_path_for_write(
                raw_path,
                root,
                ctx.sender_id,
                ctx.agent_name,
                ctx.is_clone_admin,
            )?
        } else {
            let _ = super::validate_path(raw_path)?;
            PathBuf::from(raw_path)
        };

    let content = input["content"].as_str().ok_or(CarrierError::InvalidInput(
        "Missing 'content' parameter".to_string(),
    ))?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| CarrierError::Internal(format!("Failed to create directories: {e}")))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| CarrierError::Internal(format!("Failed to write file: {e}")))?;

    // Public view URL so any clone can paste a clickable link (system capability).
    let mut msg = format!("Successfully wrote {} bytes to {}", content.len(), raw_path);
    if let (Some(an), Some(sid)) = (ctx.agent_name, ctx.sender_id) {
        if let Some(rel) = crate::file_view::rel_path_for_user_write(raw_path) {
            if let Some(url) =
                crate::file_view::build_file_view_url(ctx.external_url, an, &rel, sid)
            {
                msg.push_str(&format!(
                    "\nview_url: {url}\n(将 view_url 贴给用户即可在浏览器中打开；勿把全文粘进聊天。)"
                ));
            }
        }
    }
    Ok(msg)
}

async fn tool_file_list(input: &Value, ctx: &ToolContext<'_>) -> CarrierResult<String> {
    let raw_path = input["path"].as_str().ok_or(CarrierError::InvalidInput(
        "Missing 'path' parameter".to_string(),
    ))?;

    let resolved =
        if let (Some(hd), Some(sid), Some(an)) = (ctx.home_dir, ctx.sender_id, ctx.agent_name) {
            match resolve_user_data_path(raw_path, hd, sid, ctx.owner_id, an) {
                Some(Ok(path)) => path,
                Some(Err(e)) => return Err(e),
                None => {
                    // Internal path — go through sandbox
                    super::resolve_file_path_for_read(
                        raw_path,
                        ctx.workspace_root,
                        ctx.sender_id,
                        ctx.agent_name,
                    )?
                }
            }
        } else {
            super::resolve_file_path_for_read(
                raw_path,
                ctx.workspace_root,
                ctx.sender_id,
                ctx.agent_name,
            )?
        };

    // For user-data paths (output/ memory/), treat missing directory as empty
    let is_user_data = raw_path.starts_with("output/")
        || raw_path == "output"
        || raw_path.starts_with("memory/")
        || raw_path == "memory";

    // Friendly error: if path points to a file (not a directory), tell the
    // LLM clearly instead of returning the cryptic OS "Not a directory" error.
    if let Ok(metadata) = tokio::fs::metadata(&resolved).await {
        if metadata.is_file() {
            return Err(CarrierError::InvalidInput(format!(
                "路径 '{raw_path}' 是一个文件，不是目录。file_list 只能列出目录内容。\n\
                 修正方法：\n\
                 - 想读取这个文件内容 → 用 file_read(path=\"{raw_path}\")\n\
                 - 想列出它所在的目录 → 用 file_list 并去掉文件名（例如列出上级目录）"
            )));
        }
    }

    let read_dir_result = tokio::fs::read_dir(&resolved).await;
    let mut entries = match read_dir_result {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && is_user_data => {
            return Ok("(empty directory)".to_string());
        }
        Err(e) => {
            return Err(CarrierError::Internal(format!(
                "Failed to list directory: {e}"
            )))
        }
    };
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| CarrierError::Internal(format!("Failed to read entry: {e}")))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    if files.is_empty() {
        Ok("(empty directory)".to_string())
    } else {
        Ok(files.join("\n"))
    }
}

async fn tool_file_convert(input: &Value, ctx: &ToolContext<'_>) -> CarrierResult<String> {
    let raw_input_path = input["input_path"]
        .as_str()
        .ok_or(CarrierError::InvalidInput(
            "Missing 'input_path' parameter".to_string(),
        ))?;
    let output_format = input["output_format"]
        .as_str()
        .ok_or(CarrierError::InvalidInput(
            "Missing 'output_format' parameter".to_string(),
        ))?;
    let raw_output_path = input["output_path"].as_str();

    let input_path = super::resolve_file_path(raw_input_path, ctx.workspace_root)?;
    if !input_path.exists() {
        return Err(CarrierError::InvalidInput(format!(
            "Input file not found: {}",
            input_path.display()
        )));
    }
    let metadata = std::fs::metadata(&input_path)
        .map_err(|e| CarrierError::Internal(format!("Cannot read input file metadata: {e}")))?;
    if metadata.len() > 50 * 1024 * 1024 {
        return Err(CarrierError::InvalidInput(format!(
            "Input file too large: {} bytes (max 50MB)",
            metadata.len()
        )));
    }

    let output_path = if let Some(op) = raw_output_path {
        // User-specified output path — resolve through the same logic as file_write
        if let (Some(hd), Some(sid), Some(an)) = (ctx.home_dir, ctx.sender_id, ctx.agent_name) {
            match resolve_user_data_path(op, hd, sid, ctx.owner_id, an) {
                Some(Ok(path)) => path,
                Some(Err(e)) => return Err(e),
                None => {
                    if let Some(root) = ctx.workspace_root {
                        crate::workspace_sandbox::resolve_sandbox_path_for_write(
                            op,
                            root,
                            ctx.sender_id,
                            ctx.agent_name,
                            ctx.is_clone_admin,
                        )?
                    } else {
                        let _ = super::validate_path(op)?;
                        PathBuf::from(op)
                    }
                }
            }
        } else if let Some(root) = ctx.workspace_root {
            crate::workspace_sandbox::resolve_sandbox_path_for_write(
                op,
                root,
                ctx.sender_id,
                ctx.agent_name,
                ctx.is_clone_admin,
            )?
        } else {
            let _ = super::validate_path(op)?;
            PathBuf::from(op)
        }
    } else {
        // Auto-generated output path — use top-level senders directory
        let input_stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("converted");
        let sender = ctx.sender_id.unwrap_or("unknown");
        let agent = ctx.agent_name.unwrap_or("unknown");
        let oid = ctx.owner_id.unwrap_or(sender);
        let output_dir = if let Some(hd) = ctx.home_dir {
            types::config::sender_data_dir(hd, oid, agent, Some(sender)).join("output")
        } else {
            PathBuf::from("output")
        };
        let _ = std::fs::create_dir_all(&output_dir);
        let filename = format!("{input_stem}.{output_format}");
        output_dir.join(filename)
    };

    let mut cmd = tokio::process::Command::new("pandoc");
    cmd.arg(&input_path)
        .arg("-t")
        .arg(output_format)
        .arg("-o")
        .arg(&output_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = cmd.spawn().map_err(|e| {
        CarrierError::Internal(format!("Failed to run pandoc (is it installed?): {e}"))
    })?;

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| CarrierError::Internal("Pandoc timed out after 60 seconds".to_string()))
        .and_then(|r| {
            r.map_err(|e| CarrierError::Internal(format!("Pandoc process error: {e}")))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(CarrierError::Internal(format!(
            "Pandoc conversion failed: {stderr}"
        )));
    }

    if !output_path.exists() {
        return Err(CarrierError::Internal(
            "Pandoc completed but no output file was produced".to_string(),
        ));
    }

    let out_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(format!(
        "Successfully converted {} -> {}\nInput: {} ({} bytes)\nOutput: {} ({} bytes)",
        raw_input_path,
        output_format,
        input_path.display(),
        metadata.len(),
        output_path.display(),
        out_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/tmp/oc-fs-test-home")
    }

    #[tokio::test]
    async fn file_write_rejects_replacement_char_path() {
        // A corrupted filename (LLM emitting broken UTF-8) is un-typeable by
        // the model afterwards - every follow-up read/patch/delete fails and
        // loops (2026-08-21 86bus incident). Reject at write time.
        let ctx = crate::tool_context::ToolContext {
            kernel: None,
            memory: None,
            caller_agent_id: None,
            mcp_connections: None,
            fetch_engine: None,
            allowed_env_vars: None,
            workspace_root: None,
            brain: None,
            exec_policy: None,
            cli_exec_config: None,
            process_manager: None,
            sender_id: None,
            owner_id: None,
            home_dir: None,
            agent_name: None,
            subagent_configs: None,
            channel_type: None,
            max_tool_level: types::tool::PermissionLevel::Write,
            is_clone_admin: false,
            external_url: None,
            flow_elevated_tools: None,
            flow_shell_allow: None,
            flow_deny_tools: None,
            flow_allowed_tools: None,
        };
        let input = serde_json::json!({
            "path": "output/p/\u{FFFD}\u{FFFD}材.md",
            "content": "x",
        });
        let err = tool_file_write(&input, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("U+FFFD"), "{err}");
    }

    #[tokio::test]
    async fn file_read_enoent_on_document_extension_is_answer_not_error() {
        // 2026-08-25 ordering regression: probing output/报告.docx before
        // generating it used to fall into the markitdown path and return an
        // error — resurrecting the error-tracker pollution the ENOENT-as-
        // answer fix exists to kill. The existence gate now runs BEFORE the
        // document-format dispatch.
        let ctx = test_read_ctx();
        let out = tool_file_read(
            &serde_json::json!({"path": "oc_fs_missing_probe_98765432/报告.docx"}),
            &ctx,
        )
        .await;
        let s = out.expect("ENOENT probe must be Ok");
        assert!(s.contains(types::tool::FILE_READ_ENOENT_MARKER), "{s}");
    }

    #[tokio::test]
    async fn file_read_enotdir_is_real_error_not_missing_answer() {
        // A path THROUGH a regular file (file.txt/child) errors ENOTDIR —
        // that must surface as a real error, not the friendly "不存在"
        // answer (2026-08-25: non-NotFound metadata errors were all reported
        // as nonexistence, inviting file_write over permission-walled files).
        let real = "oc_fs_enotdir_probe_file_2468.txt";
        std::fs::write(real, "x").unwrap();
        let out = tool_file_read(
            &serde_json::json!({"path": format!("{real}/child.md")}),
            &test_read_ctx(),
        )
        .await;
        let _ = std::fs::remove_file(real);
        let msg = out.expect_err("ENOTDIR must be Err").to_string();
        assert!(
            !msg.contains(types::tool::FILE_READ_ENOENT_MARKER),
            "must not claim the file doesn't exist: {msg}"
        );
        assert!(msg.contains("无法访问"), "{msg}");
    }

    fn test_read_ctx() -> crate::tool_context::ToolContext<'static> {
        crate::tool_context::ToolContext {
            kernel: None,
            memory: None,
            caller_agent_id: None,
            mcp_connections: None,
            fetch_engine: None,
            allowed_env_vars: None,
            workspace_root: None,
            brain: None,
            exec_policy: None,
            cli_exec_config: None,
            process_manager: None,
            sender_id: None,
            owner_id: None,
            home_dir: None,
            agent_name: None,
            subagent_configs: None,
            channel_type: None,
            max_tool_level: types::tool::PermissionLevel::Write,
            is_clone_admin: false,
            external_url: None,
            flow_elevated_tools: None,
            flow_shell_allow: None,
            flow_deny_tools: None,
            flow_allowed_tools: None,
        }
    }

    #[test]
    fn input_path_resolves_to_sender_input_dir() {
        // Files the user sent are saved by the bridge into
        // senders/{sender}/input/. file_read/file_list must resolve input/
        // there (not into output/input/ as the old catch-all did).
        let h = home();
        let sender = "u1@im.wechat";
        let p = resolve_user_data_path("input/为.md", &h, sender, None, "mo-catering-ops")
            .expect("input/ should resolve (not internal)")
            .expect("path should be ok");
        let expected = h
            .join("workspaces")
            .join("mo-catering-ops")
            .join("senders")
            .join(sender)
            .join("input")
            .join("为.md");
        assert_eq!(p, expected);
    }

    #[test]
    fn input_alone_resolves_to_input_dir() {
        let h = home();
        let p = resolve_user_data_path("input", &h, "u1", None, "ag")
            .unwrap()
            .unwrap();
        assert_eq!(p, h.join("workspaces/ag/senders/u1/input"));
    }

    #[test]
    fn output_memory_and_catchall_unchanged() {
        // Regression: existing output/ / memory/ / catch-all routing must not
        // change when adding the input/ branch.
        let h = home();
        let p_out = resolve_user_data_path("output/r.md", &h, "u1", None, "ag")
            .unwrap()
            .unwrap();
        assert_eq!(p_out, h.join("workspaces/ag/senders/u1/output/r.md"));

        let p_mem = resolve_user_data_path("memory/n.md", &h, "u1", None, "ag")
            .unwrap()
            .unwrap();
        assert_eq!(p_mem, h.join("workspaces/ag/senders/u1/memory/n.md"));

        // catch-all (no recognized prefix) still goes to output/
        let p_catch = resolve_user_data_path("foo.md", &h, "u1", None, "ag")
            .unwrap()
            .unwrap();
        assert_eq!(p_catch, h.join("workspaces/ag/senders/u1/output/foo.md"));
    }

    #[test]
    fn document_extension_detects_formats() {
        use std::path::Path;
        assert_eq!(
            document_extension(Path::new("foo.pdf")),
            Some("pdf".to_string())
        );
        assert_eq!(
            document_extension(Path::new("销售.XLSX")),
            Some("xlsx".to_string())
        );
        assert_eq!(
            document_extension(Path::new("input/report.docx")),
            Some("docx".to_string())
        );
        assert_eq!(
            document_extension(Path::new("a.pptx")),
            Some("pptx".to_string())
        );
        assert_eq!(document_extension(Path::new("notes.md")), None);
        assert_eq!(document_extension(Path::new("data.csv")), None);
        assert_eq!(document_extension(Path::new("noext")), None);
    }

    #[test]
    fn directory_read_hint_steer_to_file_list() {
        let msg = directory_read_hint("output/pipeline-20260725-x");
        // The hint must name the corrective tool AND echo the path so the agent
        // can copy it — this is exactly what breaks the file_read-on-directory
        // loop. If a future cleanup makes the error generic again, this fires.
        assert!(
            msg.contains("file_list"),
            "hint must mention file_list: {msg}"
        );
        assert!(
            msg.contains("file_read"),
            "hint must mention file_read: {msg}"
        );
        assert!(
            msg.contains("output/pipeline-20260725-x"),
            "hint must echo the path: {msg}"
        );
    }
}
