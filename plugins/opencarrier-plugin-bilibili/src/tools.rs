//! Bilibili tool provider.

use crate::api;
use crate::schema::BilibiliToolSpec;
use crate::BILIBILI_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;
use std::collections::HashMap;

pub struct BilibiliToolProvider {
    spec: BilibiliToolSpec,
}

impl BilibiliToolProvider {
    fn from_spec(spec: BilibiliToolSpec) -> Self {
        Self { spec }
    }
}

impl ToolProvider for BilibiliToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(self.spec.name, self.spec.description, self.spec.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let entry = BILIBILI_TENANTS
            .get(&ctx.tenant_id)
            .ok_or_else(|| {
                let names: Vec<String> = BILIBILI_TENANTS.iter().map(|e| e.key().clone()).collect();
                PluginError::tool(format!(
                    "Unknown Bilibili tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    names.join(", ")
                ))
            })?;

        let cookie_str = api::build_cookie(
            &entry.value().sessdata,
            &entry.value().bili_jct,
            &entry.value().dede_user_id,
        );

        let mut params = (self.spec.build_params)(args);

        // Special handling for comments: need aid from bvid
        if self.spec.name == "bilibili_comments" {
            if let Some(bvid) = args["bvid"].as_str() {
                // Get aid via video info API
                let mut nav_params = HashMap::new();
                nav_params.insert("bvid".into(), bvid.into());
                if let Ok(video_resp) = api::bilibili_api_blocking(&cookie_str, reqwest::Method::GET, "/x/web-interface/view", &nav_params, false) {
                    if let Some(aid) = video_resp.pointer("/data/aid").and_then(|v| v.as_i64()) {
                        params.insert("oid".into(), aid.to_string());
                    }
                }
            }
        }

        // Special handling for subtitle: need cid from video info
        if self.spec.name == "bilibili_subtitle" {
            if let Some(bvid) = args["bvid"].as_str() {
                let mut nav_params = HashMap::new();
                nav_params.insert("bvid".into(), bvid.into());
                if let Ok(video_resp) = api::bilibili_api_blocking(&cookie_str, reqwest::Method::GET, "/x/web-interface/view", &nav_params, false) {
                    if let Some(cid) = video_resp.pointer("/data/cid").and_then(|v| v.as_i64()) {
                        params.insert("cid".into(), cid.to_string());
                    }
                }
            }
        }

        // Special handling for favorite: need media_id from folder list
        if self.spec.name == "bilibili_favorite" {
            // Get self uid
            if let Ok(uid) = api::get_self_uid_blocking(&cookie_str) {
                let mut folder_params = HashMap::new();
                folder_params.insert("up_mid".into(), uid.to_string());
                if let Ok(folder_resp) = api::bilibili_api_blocking(&cookie_str, reqwest::Method::GET, "/x/v3/fav/folder/created/list-all", &folder_params, true) {
                    if let Some(first_folder) = folder_resp.pointer("/data/list")
                        .and_then(|l| l.as_array())
                        .and_then(|a| a.first())
                    {
                        if let Some(mid) = first_folder.get("id").and_then(|v| v.as_i64()) {
                            params.insert("media_id".into(), mid.to_string());
                        }
                    }
                }
            }
        }

        // Special handling for user_videos/feed with username: resolve uid
        // (Skip for now — callers should provide numeric UIDs)

        let result = api::bilibili_api_blocking(
            &cookie_str,
            self.spec.method.clone(),
            self.spec.path,
            &params,
            self.spec.signed,
        )
        .map_err(|e| PluginError::tool(e))?;

        let parsed = (self.spec.parse_response)(&result);
        let text = serde_json::to_string(&parsed).unwrap_or_else(|_| parsed.to_string());
        Ok(api::truncate_result(text))
    }
}

pub fn build_all_tools() -> Vec<Box<dyn ToolProvider>> {
    crate::schema::all_tools()
        .into_iter()
        .map(|spec| Box::new(BilibiliToolProvider::from_spec(spec)) as Box<dyn ToolProvider>)
        .collect()
}
