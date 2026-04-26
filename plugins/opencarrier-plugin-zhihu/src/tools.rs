//! Zhihu tool provider.

use crate::api;
use crate::schema::ZhihuToolSpec;
use crate::ZHIHU_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

pub struct ZhihuToolProvider {
    spec: ZhihuToolSpec,
}

impl ZhihuToolProvider {
    fn from_spec(spec: ZhihuToolSpec) -> Self {
        Self { spec }
    }
}

impl ToolProvider for ZhihuToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(self.spec.name, self.spec.description, self.spec.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let entry = ZHIHU_TENANTS
            .get(&ctx.tenant_id)
            .ok_or_else(|| {
                let names: Vec<String> = ZHIHU_TENANTS.iter().map(|e| e.key().clone()).collect();
                PluginError::tool(format!(
                    "Unknown Zhihu tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    names.join(", ")
                ))
            })?;

        let cookie_str = entry.value().cookie.clone();

        // Substitute path params like {question_id}
        let mut path = self.spec.path.to_string();
        if let Some(obj) = args.as_object() {
            for (key, val) in obj {
                if let Some(s) = val.as_str() {
                    path = path.replace(&format!("{{{key}}}"), s);
                }
            }
        }

        let query = (self.spec.build_query)(args);

        let result = api::zhihu_api_blocking(
            &cookie_str,
            self.spec.method.clone(),
            &path,
            query.as_deref(),
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
        .map(|spec| Box::new(ZhihuToolProvider::from_spec(spec)) as Box<dyn ToolProvider>)
        .collect()
}
