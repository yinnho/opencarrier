//! Reddit tool provider — generic ToolProvider for all Reddit REST API tools.

use crate::api;
use crate::schema::RedditToolSpec;
use crate::REDDIT_TENANTS;
use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

/// Generic Reddit tool provider — one instance per tool.
pub struct RedditToolProvider {
    spec: RedditToolSpec,
}

impl RedditToolProvider {
    fn from_spec(spec: RedditToolSpec) -> Self {
        Self { spec }
    }
}

impl ToolProvider for RedditToolProvider {
    fn definition(&self) -> ToolDef {
        ToolDef::new(self.spec.name, self.spec.description, self.spec.schema.clone())
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        // 1. Look up tenant credentials
        let cookie_str = {
            let entry = REDDIT_TENANTS
                .get(&ctx.tenant_id)
                .ok_or_else(|| {
                    let names: Vec<String> =
                        REDDIT_TENANTS.iter().map(|e| e.key().clone()).collect();
                    PluginError::tool(format!(
                        "Unknown Reddit tenant '{}'. Available tenants: {}",
                        ctx.tenant_id,
                        names.join(", ")
                    ))
                })?;
            entry.value().cookie.clone()
        };

        // 2. Resolve username if needed (saved/upvoted endpoints)
        let mut resolved_args = args.clone();
        if self.spec.needs_username {
            let username = api::get_configured_username(&ctx.tenant_id)
                .or_else(|| {
                    // Try to fetch from API if not configured
                    api::get_username_blocking(&cookie_str).ok()
                })
                .ok_or_else(|| {
                    PluginError::tool(
                        "Cannot resolve Reddit username. Set 'username' in tenant config.".to_string(),
                    )
                })?;
            if let Some(m) = resolved_args.as_object_mut() {
                m.insert("_username".to_string(), Value::String(username));
            }
        }

        // 3. Resolve path — if needs_username, replace "me" with actual username
        let path = (self.spec.build_path)(&resolved_args);
        let path = if self.spec.needs_username {
            let username = resolved_args["_username"]
                .as_str()
                .unwrap_or("me");
            path.replace("/user/me/", &format!("/user/{username}/"))
        } else {
            path
        };

        // 4. Build query string
        let query = (self.spec.build_query)(&resolved_args);

        // 5. For write tools, get modhash first, then POST
        let result = if self.spec.needs_modhash {
            let modhash = api::get_modhash_blocking(&cookie_str)
                .map_err(PluginError::tool)?;

            let body_fn = self.spec.build_body
                .ok_or_else(|| PluginError::tool("Write tool missing body builder".to_string()))?;

            let body = body_fn(&resolved_args, &modhash);

            api::reddit_api_blocking(
                &cookie_str,
                self.spec.method.clone(),
                &path,
                None,
                Some(&body),
            )
            .map_err(PluginError::tool)?
        } else {
            let query_opt = if query.is_empty() {
                None
            } else {
                Some(query.as_str())
            };
            api::reddit_api_blocking(
                &cookie_str,
                self.spec.method.clone(),
                &path,
                query_opt,
                None,
            )
            .map_err(PluginError::tool)?
        };

        // 6. Parse response
        let parsed = (self.spec.parse_response)(&result);

        // 7. Serialize, truncate, return
        let text = serde_json::to_string(&parsed)
            .unwrap_or_else(|_| parsed.to_string());

        Ok(api::truncate_result(text))
    }
}

/// Build all Reddit tool providers. Called from Plugin::tools().
pub fn build_all_tools() -> Vec<Box<dyn ToolProvider>> {
    crate::schema::all_tools()
        .into_iter()
        .map(|spec| Box::new(RedditToolProvider::from_spec(spec)) as Box<dyn ToolProvider>)
        .collect()
}
