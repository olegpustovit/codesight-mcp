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
mod freshness;
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
pub(crate) mod validate;

use serde_json::Value;

pub(in crate::service) fn dispatch(
    svc: &super::CodeService,
    tool_name: &str,
    args: Value,
) -> anyhow::Result<Value> {
    let args = validate::coerce_and_validate(tool_name, args).map_err(anyhow::Error::new)?;

    // Read-path freshness gating: in strict mode, block until the repo is fresh;
    // in relaxed mode, the response is annotated post-hoc with a stale_data_warning.
    let freshness_repo = if freshness::is_read_tool(tool_name) {
        let repo = freshness::source_id(&args).map(str::to_string);
        if let Some(ref r) = repo {
            freshness::await_if_strict(svc, r);
        }
        repo
    } else {
        None
    };

    // catalog_sources is global (no source_id): block on any in-flight ingest
    // under strict mode, otherwise annotate the response with the in-flight list.
    if tool_name == "catalog_sources" {
        freshness::await_all_if_strict(svc);
    }

    let result = if let Some(res) = registry::dispatch_tool(svc, tool_name, args)? {
        res
    } else {
        return registry::not_found(svc, tool_name);
    };

    let result = if tool_name == "catalog_sources" {
        freshness::annotate_catalog_with_in_flight(svc, result)
    } else {
        result
    };

    Ok(match freshness_repo {
        Some(ref r) => freshness::annotate(svc, r, result),
        None => result,
    })
}
