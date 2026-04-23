//! WeCom tool providers — spreadsheet operations and messaging.

use opencarrier_plugin_sdk::{PluginError, PluginToolContext, ToolDef, ToolProvider};
use serde_json::Value;

use crate::token::{self, TenantEntry};

// ---------------------------------------------------------------------------
// Create Spreadsheet (App mode only)
// ---------------------------------------------------------------------------

pub struct CreateSpreadsheetTool;

impl ToolProvider for CreateSpreadsheetTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "create_spreadsheet",
            "创建企业微信表格，返回文档链接",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "表格标题" },
                    "columns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "列标题，如 [\"日期\",\"姓名\",\"金额\"]"
                    }
                },
                "required": ["title", "columns"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let title = args["title"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing title"))?;
        let columns: Vec<String> = args["columns"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(async { create_spreadsheet_async(&tenant, title, &columns).await })
        })
    }
}

async fn create_spreadsheet_async(
    tenant: &TenantEntry,
    title: &str,
    columns: &[String],
) -> Result<String, PluginError> {
    let token = tenant
        .get_access_token()
        .map_err(|e| PluginError::tool(e.to_string()))?;

    // Create document
    let body = serde_json::json!({
        "docname": title,
        "doctype": 4  // spreadsheet
    });
    let resp =
        token::wedoc_post(&tenant.http, "cgi-bin/wedoc/create_doc", &token, &body)
            .await
            .map_err(|e| PluginError::tool(e.to_string()))?;

    let url = resp["url"].as_str().unwrap_or("").to_string();
    let docid = resp["docid"].as_str().unwrap_or("").to_string();

    // Write column headers if provided
    if !columns.is_empty() && !docid.is_empty() {
        let mut cells = Vec::new();
        for (col, header) in columns.iter().enumerate() {
            cells.push(serde_json::json!({
                "col": col,
                "row": 0,
                "content": header
            }));
        }
        let update_body = serde_json::json!({
            "docid": docid,
            "cellDatas": cells
        });
        let _ = token::wedoc_post(
            &tenant.http,
            "cgi-bin/wedoc/spreadsheet/update_sheet_cell",
            &token,
            &update_body,
        )
        .await;
    }

    Ok(if url.is_empty() {
        format!("Created spreadsheet '{}' (docid: {})", title, docid)
    } else {
        format!("Created spreadsheet '{}' — {}", title, url)
    })
}

// ---------------------------------------------------------------------------
// Add Rows (App mode only)
// ---------------------------------------------------------------------------

pub struct AddRowsTool;

impl ToolProvider for AddRowsTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "add_rows",
            "向企业微信表格添加行数据",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID或URL" },
                    "rows": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "string" } },
                        "description": "行数据，如 [[\"2025-01-01\",\"张三\",\"100\"]]"
                    }
                },
                "required": ["docid", "rows"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let docid = extract_docid(args["docid"].as_str().unwrap_or(""));
        let rows: Vec<Vec<String>> = args["rows"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|row| {
                        row.as_array()
                            .map(|cells| cells.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if docid.is_empty() || rows.is_empty() {
            return Err(PluginError::tool("Missing docid or rows"));
        }

        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(async { add_rows_async(&tenant, &docid, &rows).await })
        })
    }
}

async fn add_rows_async(
    tenant: &TenantEntry,
    docid: &str,
    rows: &[Vec<String>],
) -> Result<String, PluginError> {
    let token = tenant
        .get_access_token()
        .map_err(|e| PluginError::tool(e.to_string()))?;

    let mut cells = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        for (col_idx, cell) in row.iter().enumerate() {
            cells.push(serde_json::json!({
                "col": col_idx,
                "row": row_idx + 1,  // row 0 is header
                "content": cell
            }));
        }
    }

    let body = serde_json::json!({
        "docid": docid,
        "cellDatas": cells
    });
    token::wedoc_post(
        &tenant.http,
        "cgi-bin/wedoc/spreadsheet/update_sheet_cell",
        &token,
        &body,
    )
    .await
    .map_err(|e| PluginError::tool(e.to_string()))?;

    Ok(format!("Added {} rows to spreadsheet {}", rows.len(), docid))
}

// ---------------------------------------------------------------------------
// Query Spreadsheet (App mode only)
// ---------------------------------------------------------------------------

pub struct QuerySpreadsheetTool;

impl ToolProvider for QuerySpreadsheetTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "query_spreadsheet",
            "查询企业微信表格数据",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" }
                },
                "required": ["docid"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let docid = extract_docid(args["docid"].as_str().unwrap_or(""));
        if docid.is_empty() {
            return Err(PluginError::tool("Missing docid"));
        }

        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(async { query_spreadsheet_async(&tenant, &docid).await })
        })
    }
}

async fn query_spreadsheet_async(
    tenant: &TenantEntry,
    docid: &str,
) -> Result<String, PluginError> {
    let token = tenant
        .get_access_token()
        .map_err(|e| PluginError::tool(e.to_string()))?;

    let body = serde_json::json!({ "docid": docid });
    let resp = token::wedoc_post(
        &tenant.http,
        "cgi-bin/wedoc/spreadsheet/get_sheet_data",
        &token,
        &body,
    )
    .await
    .map_err(|e| PluginError::tool(e.to_string()))?;

    let result = serde_json::to_string_pretty(&resp).unwrap_or_default();
    // Truncate at 3000 chars to avoid overwhelming the LLM context
    if result.len() > 3000 {
        Ok(format!("{}...\n(truncated)", &result[..3000]))
    } else {
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Send WeCom Message (App and Kf modes)
// ---------------------------------------------------------------------------

pub struct SendMessageTool;

impl ToolProvider for SendMessageTool {
    fn definition(&self) -> ToolDef {
        ToolDef::new(
            "send_wecom_message",
            "发送企业微信消息给指定用户（支持企业应用和微信客服模式）",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "user_id": { "type": "string", "description": "接收人UserID（企业应用为内部UserID，微信客服为external_userid）" },
                    "content": { "type": "string", "description": "消息内容" }
                },
                "required": ["user_id", "content"]
            }),
        )
    }

    fn execute(&self, args: &Value, ctx: &PluginToolContext) -> Result<String, PluginError> {
        let user_id = args["user_id"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing user_id"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| PluginError::tool("Missing content"))?;

        let tenant = crate::TOKEN_MANAGER
            .get_tenant(&ctx.tenant_id)
            .ok_or_else(|| PluginError::tool(format!("Unknown tenant: {}", ctx.tenant_id)))?;

        match &tenant.mode {
            token::WecomMode::App { .. } => {
                token::send_app_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::Kf { .. } => {
                token::send_kf_message(&tenant, user_id, content)
                    .map_err(|e| PluginError::tool(e.to_string()))?;
            }
            token::WecomMode::SmartBot { .. } => {
                return Err(PluginError::tool(
                    "SmartBot mode replies via response_url, not send_wecom_message",
                ));
            }
        }

        Ok(format!("Message sent to {}", user_id))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract docid from a potential full URL.
fn extract_docid(input: &str) -> String {
    // If it's a URL like https://doc.weixin.qq.com/.../docid, extract the docid
    if input.starts_with("http") {
        input
            .rsplit('/')
            .next()
            .unwrap_or(input)
            .to_string()
    } else {
        input.to_string()
    }
}
