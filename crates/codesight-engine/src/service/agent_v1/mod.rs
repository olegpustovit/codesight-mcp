//! Per-tool-family modules and dispatch registry for the `agent-v1` MCP surface.

mod args;
mod catalog;
mod catalog_search;
mod content;
mod context;
mod cutplan;
mod file_tree;
mod freshness;
mod graph;
mod impact;
mod index_admin;
mod ingest;
pub mod quality;
pub(crate) mod registry;
mod session_admin;
mod structure;
mod util;
pub mod validate;

use serde_json::Value;

pub(in crate::service) fn dispatch(
    svc: &super::CodeService,
    tool_name: &str,
    args: Value,
) -> anyhow::Result<Value> {
    let args = validate::coerce_and_validate(tool_name, args).map_err(anyhow::Error::new)?;

    // Read-path freshness: never block. When a reindex is mid-flight and the
    // query touches an in-flight file, the envelope carries a
    // `STALE_DATA_WARNING`. When the query is unrelated, we demote to a
    // quiet `_meta.indexing` hint so agents aren't desensitized.
    let (freshness_repo, touched) = if freshness::is_read_tool(tool_name) {
        (
            freshness::source_id(&args).map(str::to_string),
            freshness::touched_files(&args),
        )
    } else {
        (None, Vec::new())
    };

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
        Some(ref r) => freshness::annotate(svc, r, &touched, result),
        None => result,
    })
}
