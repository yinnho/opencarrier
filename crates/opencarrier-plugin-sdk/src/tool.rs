//! Tool provider trait — for plugins that expose callable tools to agents.

use serde_json::Value;

use crate::error::PluginError;
use opencarrier_types::plugin::PluginToolContext;

/// A tool definition provided by a plugin.
pub struct ToolDef {
    /// Unique tool name (must be unique across all plugins).
    pub name: String,
    /// Description shown to the LLM.
    pub description: String,
    /// JSON Schema for the tool's parameters (pre-serialized string).
    pub parameters_json: String,
}

impl ToolDef {
    /// Create a tool definition from a JSON Schema value.
    pub fn new(name: &str, description: &str, schema: Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters_json: serde_json::to_string(&schema)
                .unwrap_or_else(|_| r#"{"type":"object"}"#.to_string()),
        }
    }
}

/// A tool provider exposes a callable tool that agents can use.
///
/// # Example
///
/// ```ignore
/// struct CreateSpreadsheetTool;
///
/// impl ToolProvider for CreateSpreadsheetTool {
///     fn definition(&self) -> ToolDef {
///         ToolDef::new(
///             "create_spreadsheet",
///             "Create a new spreadsheet",
///             serde_json::json!({
///                 "type": "object",
///                 "properties": {
///                     "title": { "type": "string", "description": "Spreadsheet title" }
///                 },
///                 "required": ["title"]
///             }),
///         )
///     }
///
///     fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
///         let title = args["title"].as_str().ok_or_else(|| PluginError::tool("Missing title"))?;
///         // ... create spreadsheet via API ...
///         Ok(format!("Created spreadsheet: {}", title))
///     }
/// }
/// ```
pub trait ToolProvider: Send + Sync {
    /// Return the tool definition (name, description, parameter schema).
    fn definition(&self) -> ToolDef;

    /// Execute the tool with the given arguments and context.
    fn execute(&self, args: &Value, context: &PluginToolContext) -> Result<String, PluginError>;
}
