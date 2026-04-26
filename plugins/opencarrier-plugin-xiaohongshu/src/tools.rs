//! Xiaohongshu tool provider — generic ToolProvider for all Creator API tools.

use crate::api;
use crate::schema::XhsToolSpec;
use crate::XHS_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::{json, Value};

/// Generic Xiaohongshu tool provider — one instance per tool.
pub struct XhsToolProvider {
    spec: XhsToolSpec,
}

impl XhsToolProvider {
    fn from_spec(spec: XhsToolSpec) -> Self {
        Self { spec }
    }
}

impl ToolProvider for XhsToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(self.spec.name, self.spec.description, self.spec.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        // 1. Look up tenant cookie
        let entry = XHS_TENANTS
            .get(&ctx.tenant_id)
            .ok_or_else(|| {
                let names: Vec<String> = XHS_TENANTS.iter().map(|e| e.key().clone()).collect();
                PluginError::tool(format!(
                    "Unknown Xiaohongshu tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    names.join(", ")
                ))
            })?;

        let cookie = entry.value().cookie.clone();

        // 2. Handle composite tool (xhs_creator_notes_summary) specially
        if self.spec.name == "xhs_creator_notes_summary" {
            return execute_notes_summary(&cookie, args);
        }

        // 3. Build query params
        let query = (self.spec.param_mapper)(args);

        // 4. Build final path
        let path = if query.is_empty() {
            self.spec.path.to_string()
        } else {
            format!("{}?{}", self.spec.path, query)
        };

        // 5. Execute API call
        let result = api::xhs_api_blocking(&cookie, &path, self.spec.method.clone())
            .map_err(PluginError::tool)?;

        // 6. Parse response
        let parsed = (self.spec.parse_response)(&result);

        // 7. Serialize, truncate, return
        let text = serde_json::to_string(&parsed)
            .unwrap_or_else(|_| parsed.to_string());

        Ok(api::truncate_result(text))
    }
}

/// Execute the composite xhs_creator_notes_summary tool.
/// Fetches note list, then fetches detail for each note.
fn execute_notes_summary(cookie: &str, args: &Value) -> Result<String, PluginError> {
    let limit = args["limit"].as_i64().unwrap_or(3);

    // Step 1: Fetch note list
    let list_path = format!(
        "/api/galaxy/creator/datacenter/note/analyze/list?type=0&page_size={limit}&page_num=1"
    );

    let list_resp = api::xhs_api_blocking(cookie, &list_path, reqwest::Method::GET)
        .map_err(PluginError::tool)?;

    let notes = list_resp
        .pointer("/data/data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    // Step 2: Fetch detail for each note
    let mut summaries = Vec::new();
    for note in notes.iter().take(limit as usize) {
        let note_id = note.get("id").and_then(|v| v.as_str()).unwrap_or("");

        if note_id.is_empty() {
            summaries.push(json!({
                "id": note.get("id"),
                "title": note.get("title"),
                "detail": {"error": "Missing note id"},
            }));
            continue;
        }

        let detail_path = format!(
            "/api/galaxy/creator/datacenter/note/base?note_id={note_id}"
        );

        match api::xhs_api_blocking(cookie, &detail_path, reqwest::Method::GET) {
            Ok(detail_resp) => {
                let detail = detail_resp.pointer("/data/data").cloned().unwrap_or(json!(null));
                summaries.push(json!({
                    "id": note_id,
                    "title": note.get("title"),
                    "list_stats": {
                        "read_count": note.get("read_count"),
                        "like_count": note.get("like_count"),
                        "fav_count": note.get("fav_count"),
                        "comment_count": note.get("comment_count"),
                    },
                    "detail": detail,
                }));
            }
            Err(e) => {
                summaries.push(json!({
                    "id": note_id,
                    "title": note.get("title"),
                    "detail": {"error": e},
                }));
            }
        }
    }

    let text = serde_json::to_string(&summaries)
        .unwrap_or_else(|_| serde_json::to_string(&json!(summaries)).unwrap_or_default());

    Ok(api::truncate_result(text))
}

/// Build all Xiaohongshu tool providers. Called from Plugin::tools().
pub fn build_all_tools() -> Vec<Box<dyn ToolProvider>> {
    crate::schema::all_tools()
        .into_iter()
        .map(|spec| Box::new(XhsToolProvider::from_spec(spec)) as Box<dyn ToolProvider>)
        .collect()
}
