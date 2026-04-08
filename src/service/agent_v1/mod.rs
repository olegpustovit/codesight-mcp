//! Per-tool-family modules and dispatch registry for the `agent-v1` MCP surface.

mod args;
mod catalog;
mod catalog_search;
mod catalog_search_files;
mod content;
mod context;
mod context_bundle;
mod context_tail;
mod cross_repo_tool;
mod file_tree;
mod graph;
mod graph_tail;
mod graph_tail_b;
mod impact;
mod impact_more;
mod index_admin;
mod ingest;
mod quality;
mod quality_more;
mod reference_tool;
mod registry;
mod structure;
mod util;
mod util_graph;

use serde_json::Value;

pub(in crate::service) fn dispatch(
    svc: &super::CodeService,
    tool_name: &str,
    args: Value,
) -> anyhow::Result<Value> {
    if let Some(res) = registry::dispatch_tool(svc, tool_name, args)? {
        return Ok(res);
    }
    registry::not_found(svc, tool_name)
}
