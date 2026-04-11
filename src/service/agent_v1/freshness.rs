//! Read-path freshness gating for `agent-v1`.
//!
//! Read tools consult [`ReindexStateManager`] before running so the dispatcher can
//! either block (strict mode) or annotate the response with a `stale_data_warning`
//! (relaxed mode) when an in-flight reindex would otherwise return stale results.
//! Write/utility tools are skipped — they drive reindex state, they don't observe it.

use serde_json::{Value, json};

use crate::server_support::reindex_state::{FreshnessMode, ReindexStateManager};
use crate::service::CodeService;

/// Tools that observe indexed state and should respect freshness gating.
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

/// In strict mode, block until the repo is fresh (or the wait times out).
/// In relaxed mode, this is a no-op — staleness is reported via [`annotate`].
pub(super) fn await_if_strict(svc: &CodeService, repo: &str) {
    if svc.reindex_states().get_freshness_mode() == FreshnessMode::Strict {
        // Bounded wait so a stuck reindex never deadlocks the MCP loop.
        let _ = svc.reindex_states().await_freshness_if_strict(repo, None);
    }
}

/// In strict mode, block until every tracked repo is fresh. Used for global
/// read tools like `catalog_sources` that have no `source_id` of their own.
pub(super) fn await_all_if_strict(svc: &CodeService) {
    if svc.reindex_states().get_freshness_mode() != FreshnessMode::Strict {
        return;
    }
    for repo in svc.reindex_states().in_flight_repos() {
        let _ = svc.reindex_states().await_freshness_if_strict(&repo, None);
    }
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

/// In relaxed mode, if a reindex is in progress for `repo`, append a
/// `stale_data_warning` warning to the response envelope so MCP clients can
/// surface it. No-op in strict mode (the request blocked above).
pub(super) fn annotate(svc: &CodeService, repo: &str, response: Value) -> Value {
    annotate_with(svc.reindex_states().as_ref(), repo, response)
}

fn annotate_with(states: &ReindexStateManager, repo: &str, mut response: Value) -> Value {
    if states.get_freshness_mode() != FreshnessMode::Relaxed {
        return response;
    }
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
    fn relaxed_mode_appends_warning_when_reindex_in_progress() {
        let states = ReindexStateManager::new();
        states.mark_reindex_start("repo-x");
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-x", body);
        let warnings = annotated["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["code"], "STALE_DATA_WARNING");
    }

    #[test]
    fn relaxed_mode_skips_warning_when_fresh() {
        let states = ReindexStateManager::new();
        // never started => fresh
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-y", body);
        assert!(annotated["warnings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn strict_mode_does_not_annotate() {
        let states = ReindexStateManager::new();
        states.set_freshness_mode("strict").unwrap();
        states.mark_reindex_start("repo-z");
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-z", body);
        assert!(annotated["warnings"].as_array().unwrap().is_empty());
    }
}
