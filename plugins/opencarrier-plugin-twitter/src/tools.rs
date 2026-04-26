//! Twitter tool provider — generic ToolProvider for all Twitter GraphQL tools.

use crate::api;
use crate::schema::TwitterToolSpec;
use crate::TWITTER_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

/// Generic Twitter tool provider — one instance per tool.
pub struct TwitterToolProvider {
    spec: TwitterToolSpec,
}

impl TwitterToolProvider {
    fn from_spec(spec: TwitterToolSpec) -> Self {
        Self { spec }
    }
}

impl ToolProvider for TwitterToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(self.spec.name, self.spec.description, self.spec.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        // 1. Look up tenant credentials
        let entry = TWITTER_TENANTS
            .get(&ctx.tenant_id)
            .ok_or_else(|| {
                let names: Vec<String> = TWITTER_TENANTS.iter().map(|e| e.key().clone()).collect();
                PluginError::tool(format!(
                    "Unknown Twitter tenant '{}'. Available tenants: {}",
                    ctx.tenant_id,
                    names.join(", ")
                ))
            })?;

        let cookie_str = format!("ct0={}; auth_token={}", entry.value().ct0, entry.value().auth_token);
        let csrf_token = entry.value().ct0.clone();

        // 2. Resolve queryId
        let query_id = api::resolve_query_id(self.spec.operation);

        // 3. Build variables and features
        let variables = (self.spec.build_variables)(args);
        let features = self.spec.features
            .clone()
            .unwrap_or_else(|| api::standard_features());

        // 4. Execute GraphQL call
        let result = api::twitter_graphql_blocking(
            &cookie_str,
            &csrf_token,
            &query_id,
            self.spec.operation,
            &variables,
            &features,
            self.spec.method.clone(),
        )
        .map_err(|e| PluginError::tool(e))?;

        // 5. Parse response
        let parsed = (self.spec.parse_response)(&result);

        // 6. Serialize, truncate, return
        let text = serde_json::to_string(&parsed)
            .unwrap_or_else(|_| parsed.to_string());

        Ok(api::truncate_result(text))
    }
}

/// Build all Twitter tool providers. Called from Plugin::tools().
pub fn build_all_tools() -> Vec<Box<dyn ToolProvider>> {
    crate::schema::all_tools()
        .into_iter()
        .map(|spec| Box::new(TwitterToolProvider::from_spec(spec)) as Box<dyn ToolProvider>)
        .collect()
}
