//! Read-path freshness annotation for `agent-v1`.
//!
//! Read tools never block on reindex: if an auto-reindex is in flight for the
//! target repo, the response envelope is annotated with `STALE_DATA_WARNING`
//! so MCP clients can surface that the answer predates the latest edits.

use serde_json::{Value, json};

use crate::server_support::reindex_state::ReindexStateManager;
use crate::service::CodeService;

/// Tools that observe indexed state and should carry a staleness annotation.
const READ_TOOLS: &[&str] = &[
    "search_catalog",
    "read_content",
    "inspect_structure",
    "inspect_graph",
    "evaluate_impact",
    "evaluate_quality",
    "assemble_context",
    "search_columns",
    "check_references",
    "get_cross_repo_map",
];

pub(super) fn is_read_tool(tool: &str) -> bool {
    READ_TOOLS.contains(&tool)
}

/// Extract the `source_id` argument the read-tool envelope uses to scope a repo.
pub(super) fn source_id(args: &Value) -> Option<&str> {
    args.get("source_id").and_then(Value::as_str)
}

/// Attach an `indexing: [...]` list + `STALE_DATA_WARNING` to `catalog_sources`
/// responses so clients can see which sources are mid-reindex.
pub(super) fn annotate_catalog_with_in_flight(svc: &CodeService, mut response: Value) -> Value {
    let in_flight = svc.reindex_states().in_flight_repos();
    if in_flight.is_empty() {
        return response;
    }
    if let Value::Object(ref mut body) = response {
        if let Some(Value::Object(data)) = body.get_mut("data") {
            data.insert("indexing".to_string(), json!(in_flight));
        }
        let warnings = body
            .entry("warnings".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = warnings {
            arr.push(json!({
                "code": "STALE_DATA_WARNING",
                "message": "one or more sources are being reindexed; catalog may omit in-flight changes.",
                "indexing": in_flight,
            }));
        }
    }
    response
}

/// If a reindex is in progress for `repo`, append a `STALE_DATA_WARNING`
/// warning to the response envelope so MCP clients can surface it. No-op if
/// the repo is fresh.
pub(super) fn annotate(svc: &CodeService, repo: &str, response: Value) -> Value {
    annotate_with(svc.reindex_states().as_ref(), repo, response)
}

fn annotate_with(states: &ReindexStateManager, repo: &str, mut response: Value) -> Value {
    let status = states.get_status(repo);
    if !status.reindex_in_progress {
        return response;
    }
    if let Value::Object(ref mut body) = response {
        let warnings = body
            .entry("warnings".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = warnings {
            arr.push(json!({
                "code": "STALE_DATA_WARNING",
                "message": "reindex in progress; results may reflect a previous index state.",
                "stale_since_ms": status.stale_since_ms,
            }));
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_warning_when_reindex_in_progress() {
        let states = ReindexStateManager::new();
        states.mark_reindex_start("repo-x");
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-x", body);
        let warnings = annotated["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["code"], "STALE_DATA_WARNING");
    }

    #[test]
    fn skips_warning_when_fresh() {
        let states = ReindexStateManager::new();
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-y", body);
        assert!(annotated["warnings"].as_array().unwrap().is_empty());
    }
}
