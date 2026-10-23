//! Read-path freshness annotation for `agent-v1`.
//!
//! Read tools never block on reindex. When a reindex is mid-flight for the
//! target repo, we emit a `STALE_DATA_WARNING` *only* when the query's
//! touched files intersect the in-flight file set. Unrelated reindexes
//! degrade to a quiet `_meta.indexing` hint so agents aren't desensitized
//! by warnings that don't affect their results.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::server_support::reindex_state::{ReindexStateManager, ReindexStatus};
use crate::service::CodeService;

/// Tools that observe indexed state and should carry a staleness annotation.
/// Includes both the public 6-tool surface and the legacy handler names that
/// the MCP adapters delegate to.
const READ_TOOLS: &[&str] = &[
    "search",
    "outline",
    "read",
    "references",
    "impact",
    "health",
    "search_catalog",
    "read_content",
    "inspect_structure",
    "inspect_graph",
    "evaluate_quality",
];

pub(super) fn is_read_tool(tool: &str) -> bool {
    READ_TOOLS.contains(&tool)
}

/// Scope freshness to an explicit source, or the public tools' canonical cwd.
pub(super) fn source_id(args: &Value) -> Option<String> {
    args.get("source_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()?
                .canonicalize()
                .ok()
                .map(|path| crate::repo_id::normalize_repo_id(&path))
        })
}

/// Collect file-path-shaped args so we can intersect against the in-flight
/// set. Covers `file_path`, `file_paths[]`, `path`, `module_path`,
/// `refs[].path`, and `scope` (as a prefix marker).
pub(super) fn touched_files(args: &Value) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for key in ["file_path", "path", "module_path"] {
        if let Some(s) = args.get(key).and_then(Value::as_str)
            && !s.is_empty()
        {
            out.insert(s.to_string());
        }
    }
    for v in args
        .get("file_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(s) = v.as_str()
            && !s.is_empty()
        {
            out.insert(s.to_string());
        }
    }
    for r in args
        .get("refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(s) = r.get("path").and_then(Value::as_str)
            && !s.is_empty()
        {
            out.insert(s.to_string());
        }
        for key in ["stable_id", "id"] {
            if let Some(s) = r.get(key).and_then(Value::as_str)
                && let Some(path) = s.split("::").next()
                && !path.is_empty()
            {
                out.insert(path.to_string());
            }
        }
    }
    if let Some(s) = args.get("scope").and_then(Value::as_str)
        && !s.is_empty()
    {
        out.insert(format!("__scope__:{s}"));
    }
    out.into_iter().collect()
}

/// Attach an `indexing: [...]` list to `catalog_sources` when any source is
/// mid-reindex. The list is the machine-readable signal clients need; we
/// deliberately do *not* push a `STALE_DATA_WARNING` here because the
/// catalog request is not scoped to a single source, so "something else is
/// reindexing" is noise for callers of this endpoint. Per-tool handlers go
/// through [`annotate`] where the touched-file intersection check decides
/// whether to warn.
pub(super) fn annotate_catalog_with_in_flight(svc: &CodeService, mut response: Value) -> Value {
    let in_flight = svc.reindex_states().in_flight_repos();
    if in_flight.is_empty() {
        return response;
    }
    if let Value::Object(ref mut body) = response
        && let Some(Value::Object(data)) = body.get_mut("data")
    {
        data.insert("indexing".to_string(), json!(in_flight));
    }
    response
}

/// Annotate `response` with a reindex hint or warning depending on whether
/// the current query's `touched_files` intersects the in-flight file set.
pub(super) fn annotate(
    svc: &CodeService,
    repo: &str,
    touched: &[String],
    response: Value,
) -> Value {
    let status = svc.reindex_states().get_status(repo);
    if !status.reindex_in_progress {
        return response;
    }
    if query_intersects_in_flight(touched, &status.files_in_flight) {
        svc.savings_ledger_record_stale_warning(repo, &status.files_in_flight);
    }
    annotate_with(svc.reindex_states().as_ref(), repo, touched, response)
}

fn annotate_with(
    states: &ReindexStateManager,
    repo: &str,
    touched: &[String],
    response: Value,
) -> Value {
    let status = states.get_status(repo);
    annotate_status(repo, status, touched, response)
}

/// True if any `touched` path matches `in_flight`. A touched entry is a path
/// unless it starts with `__scope__:` in which case the payload is a scope
/// prefix that matches if *any* in-flight path starts with the prefix.
/// Empty touched falls back to true so callers that can't compute a hint
/// still receive warnings.
fn query_intersects_in_flight(touched: &[String], in_flight: &[String]) -> bool {
    if touched.is_empty() {
        return true;
    }
    for t in touched {
        if let Some(prefix) = t.strip_prefix("__scope__:") {
            if in_flight.iter().any(|f| f.starts_with(prefix)) {
                return true;
            }
        } else if in_flight.iter().any(|f| f == t) {
            return true;
        }
    }
    false
}

fn annotate_status(
    repo: &str,
    status: ReindexStatus,
    touched: &[String],
    mut response: Value,
) -> Value {
    if !status.reindex_in_progress {
        return response;
    }
    let intersects = query_intersects_in_flight(touched, &status.files_in_flight);
    let Value::Object(ref mut body) = response else {
        return response;
    };
    let files_in_flight: Vec<String> = status.files_in_flight.into_iter().take(5).collect();
    if intersects {
        let meta = body.entry("_meta").or_insert_with(|| json!({}));
        if meta["confidence"].is_null() || meta["confidence"] == "safe" {
            meta["confidence"] = json!("partial");
        }
        let warnings = body
            .entry("warnings".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = warnings {
            arr.push(json!({
                "code": "STALE_DATA_WARNING",
                "message": "reindex in progress; results may reflect a previous index state.",
                "indexing_repo": repo,
                "indexing_started_ago_ms": status.stale_since_ms,
                "files_in_flight": files_in_flight,
            }));
        }
    } else {
        let meta = body
            .entry("_meta".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(map) = meta {
            map.insert("indexing".to_string(), json!(files_in_flight));
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_stale_status(files: &[&str], stale_since_ms: u64) -> ReindexStatus {
        ReindexStatus {
            reindex_in_progress: true,
            index_stale: true,
            stale_since_ms: Some(stale_since_ms),
            files_in_flight: files.iter().map(|s| s.to_string()).collect(),
            reindex_error: None,
            reindex_failures: None,
        }
    }

    #[test]
    fn warning_when_query_intersects_in_flight() {
        // ROADMAP P2.6: a ≥500 ms in-flight reindex that overlaps the query
        // escalates to a full STALE_DATA_WARNING.
        let status = fake_stale_status(&["src/lib.rs"], 1_000);
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_status("repo-x", status, &["src/lib.rs".to_string()], body);
        let warnings = annotated["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["code"], "STALE_DATA_WARNING");
    }

    #[test]
    fn warning_when_reindex_just_started() {
        let status = fake_stale_status(&["src/lib.rs"], 0);
        let body = json!({"ok": true, "data": {}, "warnings": [], "_meta": {"confidence": "safe"}});
        let annotated = annotate_status("repo-x", status, &["src/lib.rs".to_string()], body);
        assert_eq!(annotated["warnings"][0]["code"], "STALE_DATA_WARNING");
        assert_eq!(annotated["_meta"]["confidence"], "partial");
    }

    #[test]
    fn freshness_preserves_weaker_confidence() {
        for confidence in ["partial", "heuristic", "unreliable"] {
            let body = json!({"ok": true, "_meta": {"confidence": confidence}, "warnings": []});
            let annotated =
                annotate_status("repo-x", fake_stale_status(&["src/lib.rs"], 0), &[], body);
            assert_eq!(annotated["_meta"]["confidence"], confidence);
            assert_eq!(annotated["warnings"][0]["code"], "STALE_DATA_WARNING");
        }
    }

    #[test]
    fn hint_not_warning_when_query_does_not_touch_in_flight() {
        let status = fake_stale_status(&["src/lib.rs"], 1_000);
        let body = json!({ "ok": true, "data": {}, "warnings": [], "_meta": {} });
        let annotated = annotate_status("repo-x", status, &["tests/foo.rs".to_string()], body);
        assert!(annotated["warnings"].as_array().unwrap().is_empty());
        let indexing = annotated["_meta"]["indexing"].as_array().unwrap();
        assert_eq!(indexing[0], "src/lib.rs");
    }

    #[test]
    fn skips_warning_when_fresh() {
        let states = ReindexStateManager::new();
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_with(&states, "repo-y", &[], body);
        assert!(annotated["warnings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn empty_touched_falls_back_to_warning() {
        let status = fake_stale_status(&["src/lib.rs"], 1_000);
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_status("repo-x", status, &[], body);
        assert_eq!(annotated["warnings"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn scope_prefix_intersects() {
        let status = fake_stale_status(&["src/service/mod.rs"], 1_000);
        let body = json!({ "ok": true, "data": {}, "warnings": [] });
        let annotated = annotate_status(
            "repo-x",
            status,
            &["__scope__:src/service/".to_string()],
            body,
        );
        assert_eq!(annotated["warnings"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn touched_files_collects_all_shapes() {
        let args = json!({
            "source_id": "r",
            "file_path": "a.rs",
            "file_paths": ["b.rs", "c.rs"],
            "path": "d.rs",
            "refs": [
                {"path": "e.rs"},
                {"stable_id": "f.rs::Foo#function"},
            ],
            "scope": "src/",
        });
        let got = touched_files(&args);
        assert!(got.contains(&"a.rs".to_string()));
        assert!(got.contains(&"b.rs".to_string()));
        assert!(got.contains(&"c.rs".to_string()));
        assert!(got.contains(&"d.rs".to_string()));
        assert!(got.contains(&"e.rs".to_string()));
        assert!(got.contains(&"f.rs".to_string()));
        assert!(got.contains(&"__scope__:src/".to_string()));
    }
}
