//! Static dispatch table for `agent-v1` tools (additive extension point).

use serde_json::Value;

use crate::service::CodeService;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

type AgentV1Handler = fn(&CodeService, Value) -> anyhow::Result<Value>;

static AGENT_V1_HANDLERS: &[(&str, AgentV1Handler)] = &[
    ("ingest_sources", CodeService::ingest_sources),
    ("catalog_sources", CodeService::catalog_sources),
    ("search_catalog", CodeService::search_catalog),
    ("read_content", CodeService::read_content),
    ("inspect_structure", CodeService::inspect_structure),
    ("inspect_graph", CodeService::inspect_graph),
    ("evaluate_impact", CodeService::evaluate_impact),
    ("evaluate_quality", CodeService::evaluate_quality),
    ("assemble_context", CodeService::assemble_context),
    ("manage_indexes", CodeService::manage_indexes),
    ("search_columns", CodeService::search_columns),
    ("check_references", CodeService::check_references),
    ("get_cross_repo_map", CodeService::get_cross_repo_map),
];

pub(in crate::service) fn dispatch_tool(
    svc: &CodeService,
    tool_name: &str,
    arguments: Value,
) -> anyhow::Result<Option<Value>> {
    for (name, handler) in AGENT_V1_HANDLERS {
        if *name == tool_name {
            return Ok(Some(handler(svc, arguments)?));
        }
    }
    Ok(None)
}

pub(in crate::service) fn not_found(svc: &CodeService, tool_name: &str) -> anyhow::Result<Value> {
    svc.error_response(
        &MetaBuilder::new(0),
        ToolError {
            category: ToolErrorCategory::NotFound,
            code: "NOT_FOUND",
            message: format!("Tool '{tool_name}' is not implemented."),
            detail: serde_json::json!({ "tool": tool_name }),
        },
    )
}
