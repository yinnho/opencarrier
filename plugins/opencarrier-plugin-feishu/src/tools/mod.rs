//! Feishu tool integration — generic ToolProvider for all Feishu/Lark Open API tools.

pub mod approval;
pub mod attendance;
pub mod base;
pub mod calendar;
pub mod contact;
pub mod doc;
pub mod drive;
pub mod im;
pub mod mail;
pub mod minutes;
pub mod okr;
pub mod sheets;
pub mod slides;
pub mod task;
pub mod vc;
pub mod whiteboard;
pub mod wiki;

use crate::api_ext::{feishu_api_blocking, truncate_result};
use crate::FEISHU_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use reqwest::Method;
use serde_json::Value;
use std::collections::HashMap;

/// Tool spec: defines one tool's metadata and how to map args to API params.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub method: Method,
    /// API path template with `{param}` placeholders, e.g. `"open-apis/im/v1/messages/{message_id}/reply"`.
    pub path: &'static str,
    /// Extract (path_params, query_params, body) from the tool's input args.
    /// path_params: values for `{key}` substitution in the path.
    pub param_mapper: fn(&Value) -> MappedParams,
}

/// Result of mapping tool args to API request parameters.
pub struct MappedParams {
    /// Values to substitute into `{key}` placeholders in the path.
    pub path_params: HashMap<&'static str, String>,
    /// Query string parameters.
    pub query: Option<Value>,
    /// JSON request body.
    pub body: Option<Value>,
}

/// Generic Feishu tool provider — one instance per tool.
pub struct FeishuToolProvider {
    name: String,
    description: String,
    schema: Value,
    method: Method,
    path: String,
    param_mapper: fn(&Value) -> MappedParams,
}

impl FeishuToolProvider {
    fn from_spec(spec: &ToolSpec) -> Self {
        Self {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            schema: spec.schema.clone(),
            method: spec.method.clone(),
            path: spec.path.to_string(),
            param_mapper: spec.param_mapper,
        }
    }
}

impl ToolProvider for FeishuToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(&self.name, &self.description, self.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let entry = FEISHU_TENANTS
            .get(&ctx.tenant_id)
            .ok_or_else(|| {
                let names: Vec<String> = FEISHU_TENANTS.iter().map(|e| e.key().clone()).collect();
                PluginError::tool(format!(
                    "Unknown tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    names.join(", ")
                ))
            })?;

        let mapped = (self.param_mapper)(args);

        // Substitute path parameters
        let mut resolved_path = self.path.clone();
        for (key, val) in &mapped.path_params {
            resolved_path = resolved_path.replace(&format!("{{{key}}}"), val);
        }

        let http = entry.token_cache.http().clone();
        let token_cache = entry.token_cache.clone();
        let method = self.method.clone();
        let query = mapped.query.clone();
        let body = mapped.body.clone();

        let result = feishu_api_blocking(&http, &token_cache, method, &resolved_path, query.as_ref(), body.as_ref())
            .map_err(|e| PluginError::tool(e))?;

        let text = serde_json::to_string(&result)
            .unwrap_or_else(|_| result.to_string());

        Ok(truncate_result(text))
    }
}

/// Build all Feishu tool providers. Called from Plugin::tools().
pub fn build_all_tools() -> Vec<Box<dyn ToolProvider>> {
    let mut specs: Vec<ToolSpec> = Vec::new();
    specs.extend(im::tools());
    specs.extend(doc::tools());
    specs.extend(sheets::tools());
    specs.extend(base::tools());
    specs.extend(calendar::tools());
    specs.extend(drive::tools());
    specs.extend(contact::tools());
    specs.extend(task::tools());
    specs.extend(mail::tools());
    specs.extend(vc::tools());
    specs.extend(wiki::tools());
    specs.extend(approval::tools());
    specs.extend(okr::tools());
    specs.extend(attendance::tools());
    specs.extend(slides::tools());
    specs.extend(minutes::tools());
    specs.extend(whiteboard::tools());

    specs
        .into_iter()
        .map(|spec| Box::new(FeishuToolProvider::from_spec(&spec)) as Box<dyn ToolProvider>)
        .collect()
}

// ---------------------------------------------------------------------------
// Shared param mappers
// ---------------------------------------------------------------------------

/// Helper to create an empty MappedParams with just a body.
pub fn body_only(args: &Value) -> MappedParams {
    MappedParams {
        path_params: HashMap::new(),
        query: None,
        body: Some(args.clone()),
    }
}

/// Helper to create an empty MappedParams with just query params.
pub fn query_only(args: &Value) -> MappedParams {
    MappedParams {
        path_params: HashMap::new(),
        query: Some(args.clone()),
        body: None,
    }
}
