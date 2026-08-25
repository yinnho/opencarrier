//! WeChat Official Account plugin tools — built-in, no FFI.

use serde_json::Value;
use tracing::{info, warn};
use types::error::CarrierResult;
use types::plugin::PluginToolContext;
use types::tool::{PluginToolDef, PluginToolError, ToolProvider};

use crate::api;

/// Resolve a path that may be absolute or relative to `~/.opencarrier`.
fn resolve_path(p: &str) -> std::path::PathBuf {
    if p.starts_with('/') {
        std::path::PathBuf::from(p)
    } else {
        types::config::home_dir().join(p)
    }
}

/// Returns true if the error indicates an expired/invalid access_token (WeChat errcode 40001).
pub(crate) fn is_token_expired(err: &str) -> bool {
    err.contains("40001")
}

/// Get a fresh access_token. If a prior call failed with 40001, call this to
/// invalidate the cache and get a new token for one retry.
pub(crate) async fn refresh_token(
    account: &crate::channel::OaAccountState,
) -> CarrierResult<String> {
    account.invalidate_token().await;
    account.get_token().await
}

// ---------------------------------------------------------------------------
// Draft box read tool (AI + API pattern — no MCP)
// ---------------------------------------------------------------------------

/// List/read drafts from a WeChat OA draft box. Read-only counterpart to
/// [`WeixinOaPublishArticleTool`]. Credentials are resolved server-side from
/// `senders/<app_id>/session.json` (or the caller-provided user-profile path)
/// and never pass through LLM output — so the web_fetch taint guard that
/// correctly blocks secret-bearing URLs is not triggered (2026-08-23 86bus:
/// the agent wanted to check an existing draft but had no tool and no safe
/// credential path).
pub struct WeixinOaDraftListTool;

impl ToolProvider for WeixinOaDraftListTool {
    fn definition(&self) -> PluginToolDef {
        PluginToolDef {
            name: "oa_draft_list".to_string(),
            description: "List or read drafts in a WeChat Official Account draft box (草稿箱). Returns title/digest/media_id/update_time per draft; with no_content=false also returns article HTML. Credentials are resolved server-side for server-bound accounts — do NOT put app_secret in any URL.".to_string(),
            parameters_json: r#"{"type":"object","properties":{"app_id":{"type":"string","description":"Target OA app_id (e.g. wx4e35...)"},"offset":{"type":"integer","default":0,"description":"Pagination offset"},"count":{"type":"integer","default":5,"maximum":20,"description":"Drafts per page (1-20)"},"no_content":{"type":"boolean","default":true,"description":"true = metadata only (cheap); false = include article HTML"},"title_filter":{"type":"string","description":"Only return drafts whose title contains this substring"}},"required":["app_id"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        args: &Value,
        context: &PluginToolContext,
    ) -> Result<String, PluginToolError> {
        let app_id = args["app_id"]
            .as_str()
            .ok_or_else(|| PluginToolError::tool("missing app_id"))?
            .to_string();
        let offset = args["offset"].as_u64().unwrap_or(0) as u32;
        let count = args["count"].as_u64().unwrap_or(5).clamp(1, 20) as u32;
        let no_content = args["no_content"].as_bool().unwrap_or(true);
        let title_filter = args["title_filter"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Server-bound account only: credentials come from the registered OA
        // session file. Unlike publish (which accepts user-profile secrets
        // passed by the marker bridge), a read tool has no need for
        // caller-supplied secrets — keeping them out means the LLM never
        // handles credential material here at all.
        let home = types::config::home_dir();
        let account = wechat_oa::session::load_account(&home, &app_id).ok_or_else(|| {
            PluginToolError::tool(format!(
                "没有找到公众号 {app_id} 的服务端绑定账号（senders/{app_id}/session.json 不存在或不是 weixin-oa 渠道）。只有后台绑定的公众号才能读草稿箱。"
            ))
        })?;
        // Cross-clone isolation: only the agent the OA is bound to may read
        // its draft box. publish.rs applies the same gate on the server-bound
        // fallback (bind_agent == agent_id, c6839ac); without it here, any
        // clone on a shared server could read another clone's drafts by
        // simply passing its app_id. Deny-by-default: no/empty bind_agent or
        // empty caller context also fails closed.
        let caller = context.agent_id.as_str();
        if !account_readable_by(account.bind_agent.as_deref(), caller) {
            return Err(PluginToolError::tool(format!(
                "公众号 {app_id} 没有绑定到当前分身（bind_agent 不匹配），无权读取它的草稿箱。\
                 只有后台把这个公众号绑定到本分身时才能用 oa_draft_list 读它。"
            )));
        }
        let app_secret = account.app_secret;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| PluginToolError::tool(format!("http client: {e}")))?;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginToolError::tool(format!("runtime error: {e}")))?;

        rt.block_on(async move {
            let mut token = wechat_oa::token::get_token(&http, &app_id, &app_secret)
                .await
                .map_err(|e| PluginToolError::tool(e.to_string()))?;

            // The client-side title_filter reads content.news_item, which
            // WeChat OMITS when no_content=1 — so a filter forces us to fetch
            // content (filter_and_strip strips it again afterwards if the
            // caller only wanted metadata).
            let fetch_no_content = no_content && title_filter.is_none();
            let mut page = match api::draft_batchget(&http, &token, offset, count, fetch_no_content)
                .await
            {
                Ok(v) => v,
                Err(e) if is_token_expired(&e.to_string()) => {
                    wechat_oa::token::invalidate(&app_id);
                    token = wechat_oa::token::get_token(&http, &app_id, &app_secret)
                        .await
                        .map_err(|e| PluginToolError::tool(e.to_string()))?;
                    api::draft_batchget(&http, &token, offset, count, fetch_no_content)
                        .await
                        .map_err(|e| PluginToolError::tool(e.to_string()))?
                }
                Err(e) => return Err(PluginToolError::tool(e.to_string())),
            };

            filter_and_strip(&mut page, title_filter.as_deref(), no_content);
            Ok(page.to_string())
        })
    }
}

/// Returns true if [`WeixinOaDraftListTool`] may read this account's drafts.
/// Deny-by-default: no bind_agent or empty caller context fails closed.
fn account_readable_by(bind_agent: Option<&str>, caller_agent_id: &str) -> bool {
    !caller_agent_id.is_empty() && bind_agent == Some(caller_agent_id)
}

/// Client-side post-processing of a draft_batchget page: apply the optional
/// title filter, then strip heavy fields the LLM doesn't need (cover crop
/// lists, temp-key URLs) — or strip the whole `content` block when the caller
/// wanted metadata only (content was fetched solely to run the filter).
fn filter_and_strip(page: &mut Value, title_filter: Option<&str>, strip_content: bool) {
    if let Some(f) = title_filter {
        if let Some(items) = page.get_mut("item").and_then(|i| i.as_array_mut()) {
            items.retain(|it| {
                it["content"]["news_item"]
                    .as_array()
                    .map(|news| {
                        news.iter()
                            .any(|n| n["title"].as_str().is_some_and(|t| t.contains(f)))
                    })
                    .unwrap_or(false)
            });
        }
    }
    if let Some(items) = page.get_mut("item").and_then(|i| i.as_array_mut()) {
        for it in items.iter_mut() {
            if strip_content {
                if let Some(obj) = it.as_object_mut() {
                    obj.remove("content");
                }
            } else if let Some(news) = it.get_mut("content").and_then(|c| c["news_item"].as_array_mut())
            {
                for n in news.iter_mut() {
                    if let Some(obj) = n.as_object_mut() {
                        obj.remove("cover_info");
                        obj.remove("url");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Publish article tool (AI + API pattern — no MCP)
// ---------------------------------------------------------------------------

/// Publish a formatted HTML article to a WeChat OA: resolve a cover, create a
/// draft, and optionally submit it for publishing. Driven by the bridge's
/// `[PUBLISH:app_id]` marker handler, so no agent tool-chain is involved.
pub struct WeixinOaPublishArticleTool;

impl ToolProvider for WeixinOaPublishArticleTool {
    fn definition(&self) -> PluginToolDef {
        PluginToolDef {
            name: "weixin_oa_publish_article".to_string(),
            description: "Publish a formatted HTML article to a WeChat Official Account: resolve a cover (upload the given cover_path, else fall back to the first image in the material library), create a draft, and optionally submit it for publishing. Credentials are resolved from the registered OA account for app_id.".to_string(),
            parameters_json: r#"{"type":"object","properties":{"app_id":{"type":"string","description":"Target OA app_id"},"html_path":{"type":"string","description":"Path to the WeChat-ready HTML article (absolute or relative to ~/.opencarrier)"},"title":{"type":"string","description":"Article title"},"author":{"type":"string","description":"Article author (作者). Usually resolved from the article's META_AUTHOR field; if omitted WeChat leaves the author blank."},"cover_path":{"type":"string","description":"Optional path to a generated cover image. If omitted/upload fails, falls back to the first image in the material library."},"publish":{"type":"boolean","default":true,"description":"Submit the draft for publishing immediately after creation."},"digest":{"type":"string","description":"Optional article digest/summary (摘要). Usually resolved from META_DIGEST; if both omitted, WeChat auto-extracts from the article beginning."}},"required":["app_id","html_path","title"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        args: &Value,
        _context: &PluginToolContext,
    ) -> Result<String, PluginToolError> {
        let app_id = args["app_id"]
            .as_str()
            .ok_or_else(|| PluginToolError::tool("missing app_id"))?
            .to_string();
        // app_secret comes from the user's own profile (multi-user: each user's
        // OA credentials live in their own directory). Required — without it we
        // can't get an access_token.
        let app_secret = args["app_secret"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                PluginToolError::tool(
                    "用户资料里没有这个公众号的凭证(app_secret 缺失),请先把公众号 app_id+app_secret 告诉我,我存到你资料里后再发".to_string(),
                )
            })?
            .to_string();
        let html_path = args["html_path"]
            .as_str()
            .ok_or_else(|| PluginToolError::tool("missing html_path"))?
            .to_string();
        let title = args["title"]
            .as_str()
            .ok_or_else(|| PluginToolError::tool("missing title"))?
            .to_string();
        let cover_path = args["cover_path"].as_str().map(|s| s.to_string());
        let publish = args["publish"].as_bool().unwrap_or(true);
        let digest = args["digest"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let author = args["author"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Build a fresh HTTP client; tokens flow through the central
        // `wechat-oa` core cache (keyed by app_id) — no WEIXIN_OA_STATE
        // registration needed and repeat publishes hit the cache.
        let http = reqwest::Client::new();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginToolError::tool(format!("runtime error: {e}")))?;

        rt.block_on(async move {
            let mut token = wechat_oa::token::get_token(&http, &app_id, &app_secret)
                .await
                .map_err(|e| PluginToolError::tool(e.to_string()))?;

            // --- Resolve cover (mandatory — WeChat publish requires one) ---
            // Tier a: upload the generated cover_path. Tier b: first image in
            // the material library. Both fail → abort (no coverless publish).
            let mut thumb_media_id: Option<String> = None;
            let mut cover_source = "none";

            if let Some(cp) = &cover_path {
                let resolved = resolve_path(cp);
                match std::fs::read(&resolved) {
                    Ok(bytes) => {
                        let filename = resolved
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("cover.png")
                            .to_string();
                        match api::upload_media_permanent(&http, &token, bytes, &filename).await {
                            Ok((mid, _)) => {
                                thumb_media_id = Some(mid);
                                cover_source = "generated";
                            }
                            Err(e) => warn!(error = %e, cover = %resolved.display(), "cover upload failed, falling back to material library"),
                        }
                    }
                    Err(e) => warn!(error = %e, cover = %resolved.display(), "cover file unreadable, falling back to material library"),
                }
            }

            if thumb_media_id.is_none() {
                match api::list_materials(&http, &token, "image", 0, 1).await {
                    Ok(items) => {
                        if let Some((mid, _url)) = items.first() {
                            thumb_media_id = Some(mid.clone());
                            cover_source = "library";
                            info!(media_id = %mid, "Using material-library image as cover");
                        }
                    }
                    Err(e) => warn!(error = %e, "list_materials cover fallback failed"),
                }
            }

            let thumb = thumb_media_id.ok_or_else(|| {
                PluginToolError::tool(
                    "封面生成失败且素材库无可用图片,无法发布(WeChat 发布必须有封面)".to_string(),
                )
            })?;

            // --- Read article HTML ---
            let resolved_html = resolve_path(&html_path);
            let content = std::fs::read_to_string(&resolved_html)
                .map_err(|e| PluginToolError::tool(format!("failed to read article {resolved_html:?}: {e}")))?;

            // --- Create draft (token retry on 40001) ---
            let draft_media_id = match api::add_draft(
                &http, &token, &title, &content, Some(&thumb), author.as_deref(), digest.as_deref(),
            )
            .await
            {
                Ok(mid) => mid,
                Err(e) if is_token_expired(&e.to_string()) => {
                    wechat_oa::token::invalidate(&app_id);
                    token = wechat_oa::token::get_token(&http, &app_id, &app_secret)
                        .await
                        .map_err(|e| PluginToolError::tool(e.to_string()))?;
                    api::add_draft(&http, &token, &title, &content, Some(&thumb), author.as_deref(), digest.as_deref())
                        .await
                        .map_err(|e| PluginToolError::tool(e.to_string()))?
                }
                Err(e) => return Err(PluginToolError::tool(e.to_string())),
            };
            info!(draft_media_id = %draft_media_id, "Draft created");

            // --- Publish (token retry on 40001) ---
            // Soft-fail: if the draft was created but freepublish fails (e.g.
            // 48001 "api unauthorized" — account isn't a verified service
            // account), return the draft media_id + the error so the caller
            // can tell the user to publish manually from the OA backend. Don't
            // discard the successfully-created draft by hard-erroring.
            let mut publish_id = None;
            let mut publish_error = None;
            if publish {
                match api::freepublish_submit(&http, &token, &draft_media_id).await {
                    Ok(pid) => publish_id = Some(pid),
                    Err(e) if is_token_expired(&e.to_string()) => {
                        wechat_oa::token::invalidate(&app_id);
                        match wechat_oa::token::get_token(&http, &app_id, &app_secret).await {
                            Ok(new_tok) => {
                                match api::freepublish_submit(&http, &new_tok, &draft_media_id).await {
                                    Ok(pid) => publish_id = Some(pid),
                                    Err(e2) => publish_error = Some(e2.to_string()),
                                }
                            }
                            Err(e2) => publish_error = Some(e2.to_string()),
                        }
                    }
                    Err(e) => publish_error = Some(e.to_string()),
                }
            }

            let status = if publish_id.is_some() {
                "published"
            } else if publish_error.is_some() {
                "draft_created_publish_failed"
            } else {
                "draft"
            };
            if let Some(ref err) = publish_error {
                warn!(draft_media_id = %draft_media_id, error = %err, "Draft created but freepublish failed (account may lack publish permission, e.g. 48001)");
            }
            info!(draft_media_id = %draft_media_id, publish_id = ?publish_id, cover_source, status, "Article publish completed");

            // Track the submitted publish for the daemon's zero-LLM PublishPoll
            // arm — but only for server-bound accounts (a senders/<app_id>
            // session exists): user-profile accounts have no credentials the
            // poller could use, so tracking them would strand forever-pending
            // ids that never resolve and never let the poller self-delete.
            if let Some(ref pid) = publish_id {
                let home = types::config::home_dir();
                if wechat_oa::session::load_account(&home, &app_id).is_some() {
                    if let Err(e) = wechat_oa::publish_tracker::track(&home, &app_id, pid) {
                        warn!(error = %e, publish_id = %pid, "publish_tracker track failed (poll arm will not see this publish)");
                    }
                } else {
                    info!(app_id = %app_id, publish_id = %pid, "user-profile account: publish status not tracked");
                }
            }

            Ok(serde_json::json!({
                "media_id": draft_media_id,
                "publish_id": publish_id,
                "publish_error": publish_error,
                "cover_source": cover_source,
                "status": status,
            })
            .to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_denies_cross_clone_and_defaults_closed() {
        // Same clone → readable
        assert!(account_readable_by(Some("agent-a"), "agent-a"));
        // Cross-clone → denied (the P0: clone B passing clone A's app_id)
        assert!(!account_readable_by(Some("agent-a"), "agent-b"));
        // Legacy session without bind_agent → closed
        assert!(!account_readable_by(None, "agent-a"));
        // Empty caller context (no inbound agent) → closed
        assert!(!account_readable_by(Some("agent-a"), ""));
    }

    fn page_with(titles: &[&str]) -> Value {
        let items: Vec<Value> = titles
            .iter()
            .map(|t| {
                let mut news = serde_json::json!({ "title": t });
                news["cover_info"] = serde_json::json!({"crop_list": [1, 2]});
                news["url"] = serde_json::json!("https://temp/key");
                serde_json::json!({ "content": { "news_item": [news] }, "media_id": "m" })
            })
            .collect();
        serde_json::json!({ "item": items, "total_count": items.len() })
    }

    #[test]
    fn title_filter_with_strip_content_filters_then_strips() {
        // The P2 regression: filter + metadata-only must not return an EMPTY
        // list — the caller fetched content only to run the filter.
        let mut page = page_with(&["白云素材周报", "别的文章"]);
        filter_and_strip(&mut page, Some("白云"), true);
        let items = page["item"].as_array().unwrap();
        assert_eq!(items.len(), 1, "matching draft survives the filter");
        assert!(
            items[0].get("content").is_none(),
            "content block stripped for metadata-only caller"
        );
    }

    #[test]
    fn no_filter_strips_only_heavy_fields() {
        let mut page = page_with(&["a", "b"]);
        filter_and_strip(&mut page, None, false);
        let items = page["item"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        for it in items {
            let news = &it["content"]["news_item"][0];
            assert!(news.get("cover_info").is_none());
            assert!(news.get("url").is_none());
            assert!(news.get("title").is_some(), "title kept when content kept");
        }
    }

    #[test]
    fn filter_no_match_drops_all() {
        let mut page = page_with(&["a"]);
        filter_and_strip(&mut page, Some("zzz"), false);
        assert_eq!(page["item"].as_array().map(|a| a.len()), Some(0));
    }
}
