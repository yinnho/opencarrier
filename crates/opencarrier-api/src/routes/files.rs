//! File upload and agent file management endpoints.

use crate::routes::state::AppState;
use crate::routes::common::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// GET /api/agents/{id}/files — List workspace identity files.
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let (_agent_id, entry) = match parse_and_get_agent_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        let path = workspace.join(name);
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}
/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}
/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
///
/// Immutable files (SOUL.md) cannot be overwritten once created.
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path((id, filename)): Path<(String, String)>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    // Immutable files: cannot be overwritten once created
    if IMMUTABLE_IDENTITY_FILES.contains(&filename.as_str()) {
        let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };
        if let Some(ref workspace) = entry.manifest.workspace {
            let file_path = workspace.join(&*filename);
            if file_path.exists() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": format!("{} is immutable — it cannot be overwritten after creation. \
                        This file defines the clone's identity and must not be tampered with.", filename)
                    })),
                );
            }
        }
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "File content too large (max 32KB)"})),
        );
    }

    let entry = match get_agent_or_404(&state.kernel.registry, &agent_id) { Ok(e) => e, Err(r) => return r };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };

    let file_path = workspace.join(&filename);
    // For new files, check the parent directory instead
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        // Parent must be inside workspace
        file_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = workspace.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Write failed: {e}")})),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Rename failed: {e}")})),
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}
// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------

/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}
/// Maximum upload size: 10 MB.
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;
/// Allowed content type prefixes for upload.
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/", "text/", "application/pdf", "audio/"];

fn is_allowed_content_type(ct: &str) -> bool {
    ALLOWED_CONTENT_TYPES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
}
/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts raw body bytes. The client must set:
/// - `Content-Type` header (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `X-Filename` header (original filename)
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    extensions: axum::http::Extensions,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let ctx = get_tenant_ctx(&extensions);
    // Validate agent ID format and tenant ownership
    let agent_id = match parse_agent_id_with_tenant(&id, &state.kernel.registry, &ctx) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Extract content type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf"}),
            ),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Extract sender_id from header (used to place file in per-user input dir)
    let sender_id = headers
        .get("X-Sender-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Validate size
    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": format!("File too large (max {} MB)", MAX_UPLOAD_SIZE / (1024 * 1024))}),
            ),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty file body"})),
        );
    }

    // Generate file ID
    let file_id = uuid::Uuid::new_v4().to_string();

    // Determine save location: prefer workspace users/{sender_id}/input/
    // Fall back to temp dir if no workspace or no sender_id
    let (file_path, upload_dir) = if let Some(ref sid) = sender_id {
        if let Some(entry) = state.kernel.registry.get(agent_id) {
            if let Some(ref ws) = entry.manifest.workspace {
                let user_input_dir = ws.join("users").join(sid).join("input");
                if let Err(e) = std::fs::create_dir_all(&user_input_dir) {
                    tracing::warn!("Failed to create user input dir: {e}");
                } else {
                    let dest = user_input_dir.join(&filename);
                    if let Err(e) = std::fs::write(&dest, &body) {
                        tracing::warn!("Failed to write upload to workspace: {e}");
                    } else {
                        tracing::info!(agent = %id, sender = %sid, file = %filename, "File uploaded to user input dir");
                        // Also save to temp for image resolution (attachments flow)
                        let tmp_dir = std::env::temp_dir().join("opencarrier_uploads");
                        let _ = std::fs::create_dir_all(&tmp_dir);
                        let tmp_path = tmp_dir.join(&file_id);
                        let _ = std::fs::write(&tmp_path, &body);
                        let size = body.len();
                        UPLOAD_REGISTRY.insert(
                            file_id.clone(),
                            UploadMeta {
                                content_type: content_type.clone(),
                            },
                        );

                        // Auto-transcribe audio uploads using the media engine
                        let transcription = if content_type.starts_with("audio/") {
                            let attachment = opencarrier_types::media::MediaAttachment {
                                media_type: opencarrier_types::media::MediaType::Audio,
                                mime_type: content_type.clone(),
                                source: opencarrier_types::media::MediaSource::FilePath {
                                    path: dest.to_string_lossy().to_string(),
                                },
                                size_bytes: size as u64,
                            };
                            match state
                                .kernel
                                .services
                                .media_engine
                                .transcribe_audio(&attachment)
                                .await
                            {
                                Ok(result) => {
                                    tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                                    Some(result.description)
                                }
                                Err(e) => {
                                    tracing::warn!("Audio transcription failed: {e}");
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        return (
                            StatusCode::CREATED,
                            Json(serde_json::json!(UploadResponse {
                                file_id,
                                filename,
                                content_type,
                                size,
                                transcription,
                            })),
                        );
                    }
                }
            }
        }
        // Fallback: temp dir
        let upload_dir = std::env::temp_dir().join("opencarrier_uploads");
        (upload_dir.join(&file_id), upload_dir)
    } else {
        // No sender_id — use temp dir (legacy behavior)
        let upload_dir = std::env::temp_dir().join("opencarrier_uploads");
        (upload_dir.join(&file_id), upload_dir)
    };

    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to create upload directory"})),
        );
    }

    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save file"})),
        );
    }

    let size = body.len();
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            content_type: content_type.clone(),
        },
    );

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = opencarrier_types::media::MediaAttachment {
            media_type: opencarrier_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: opencarrier_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state
            .kernel
            .services
            .media_engine
            .transcribe_audio(&attachment)
            .await
        {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}
/// GET /api/uploads/{file_id} — Serve an uploaded file.
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let file_path = std::env::temp_dir()
        .join("opencarrier_uploads")
        .join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => m.content_type.clone(),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            "image/png".to_string()
        }
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}



/// Build a router with all routes for this module.
pub fn router() -> axum::Router<std::sync::Arc<crate::routes::state::AppState>> {
    use axum::routing;
    axum::Router::new().route("/api/agents/{id}/files", routing::get(list_agent_files))
        .route("/api/agents/{id}/files/{filename}", routing::put(set_agent_file).get(get_agent_file))
        .route("/api/agents/{id}/upload", routing::post(upload_file))
        .route("/api/uploads/{file_id}", routing::get(serve_upload))
}
