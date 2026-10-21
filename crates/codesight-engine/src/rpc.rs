//! `rmcp` `ServerHandler` implementation for `CodeService`. The binary
//! wraps this with stdio + SIGINT/SIGTERM drain.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rmcp::{
    ErrorData, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
        Tool, ToolsCapability,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task;
use tokio::time::timeout;

use crate::config::{BUILD_VERSION, defaults::DISPATCH_CONCURRENCY};
use crate::service::CodeService;
use crate::service::agent_v1::registry;
use crate::service::agent_v1::validate::tool_definitions;
use crate::service::helpers::{ToolError, ToolErrorCategory};

static DISPATCH_SEM: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(DISPATCH_CONCURRENCY)));

static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(build_tool_catalog);

fn build_tool_catalog() -> Vec<Tool> {
    let raw = tool_definitions();
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(|s| Cow::Owned(s.to_string()));
        let input_schema = entry
            .get("inputSchema")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        out.push(Tool::new_with_raw(
            Cow::Owned(name),
            description,
            Arc::new(input_schema),
        ));
    }
    out
}

impl ServerHandler for CodeService {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::default();
        let mut tools = ToolsCapability::default();
        tools.list_changed = Some(false);
        capabilities.tools = Some(tools);
        ServerConfig::new(capabilities)
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new(env!("CARGO_PKG_NAME"), BUILD_VERSION))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(TOOLS.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool_name = request.name.to_string();
        let arguments = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let payload = dispatch_tool(self, tool_name, arguments).await?;
        let is_error = payload.get("error").is_some();
        let body = serde_json::to_string(&payload).map_err(|err| {
            ErrorData::internal_error(format!("failed to serialize tool response: {err}"), None)
        })?;
        let mut result = CallToolResult::default();
        result.content = vec![ContentBlock::text(body)];
        result.is_error = Some(is_error);
        Ok(result.into())
    }
}

async fn dispatch_tool(
    service: &CodeService,
    tool: String,
    arguments: Value,
) -> Result<Value, ErrorData> {
    let permit = DISPATCH_SEM.clone().acquire_owned().await.map_err(|err| {
        ErrorData::internal_error(format!("dispatch semaphore closed: {err}"), None)
    })?;
    let budget_ms = registry::timeout_ms(&tool);
    let svc = service.shared_handle().ok_or_else(|| {
        ErrorData::internal_error(
            "CodeService Arc handle missing; call install_arc_handle before serving".to_string(),
            None,
        )
    })?;
    let tool_for_err = tool.clone();
    let work = task::spawn_blocking(move || svc.dispatch(&tool, arguments));
    let joined = if budget_ms == 0 {
        work.await
    } else {
        match timeout(Duration::from_millis(budget_ms), work).await {
            Ok(inner) => inner,
            Err(_) => {
                drop(permit);
                return Ok(tool_error_payload(&tool_for_err, budget_ms));
            }
        }
    };
    drop(permit);
    let payload = joined.map_err(|join_err| {
        ErrorData::internal_error(
            format!("dispatch worker crashed for '{tool_for_err}': {join_err}"),
            None,
        )
    })?;
    payload.map_err(|err| {
        ErrorData::internal_error(format!("tool '{tool_for_err}' failed: {err}"), None)
    })
}

fn tool_error_payload(tool: &str, budget_ms: u64) -> Value {
    let err = ToolError::new(
        ToolErrorCategory::Transient,
        "TIMEOUT",
        format!("handler '{tool}' exceeded {budget_ms}ms timeout"),
        json!({
            "tool": tool,
            "budget_ms": budget_ms,
            "hint": "narrow the query (scope, unit_kinds, limit), or if a reindex is in progress wait for it to settle and retry.",
        }),
    );
    json!({
        "ok": false,
        "error": {
            "category": format!("{:?}", err.category).to_ascii_lowercase(),
            "code": err.code,
            "message": err.message,
            "detail": err.detail,
        }
    })
}
