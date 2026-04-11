//! S4-P11 — pin the `_meta` envelope shape so future handler edits cannot
//! silently drop a field that operators (or the `check --savings` ledger)
//! depend on.
//!
//! We only assert the *key set*, not the values. Timing, counters, and cost
//! numbers are inherently unstable.

use std::collections::BTreeSet;
use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

const BASELINE_META_KEYS: &[&str] = &[
    "build_sha",
    "build_time",
    "build_version",
    "cost_avoided",
    "naive_tokens",
    "powered_by",
    "response_tokens",
    "retrieval_engine",
    "retrieval_version",
    "timing_ms",
    "tokens_saved",
    "total_cost_avoided",
    "total_tokens_saved",
];

fn meta_keys(response: &Value) -> BTreeSet<String> {
    response["_meta"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn assert_has_baseline_keys(keys: &BTreeSet<String>, tool: &str) {
    for required in BASELINE_META_KEYS {
        assert!(
            keys.contains(*required),
            "{tool}: _meta missing required key `{required}`; got {keys:?}"
        );
    }
}

fn build_service() -> (CodeService, tempfile::TempDir) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    (svc, ws)
}

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

#[test]
fn ingest_sources_meta_has_baseline_keys() {
    let (svc, ws) = build_service();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("src").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    assert_has_baseline_keys(&meta_keys(&r), "ingest_sources");
}

#[test]
fn search_catalog_meta_has_baseline_keys() {
    let (svc, ws) = build_service();
    let sid = ingest(&svc, &ws.path().join("src"));

    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "search_catalog failed: {r:?}");
    assert_has_baseline_keys(&meta_keys(&r), "search_catalog");
}

#[test]
fn catalog_sources_meta_has_baseline_keys() {
    let (svc, _ws) = build_service();
    let r = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "catalog_sources failed: {r:?}");
    assert_has_baseline_keys(&meta_keys(&r), "catalog_sources");
}

#[test]
fn error_response_carries_error_category_in_meta() {
    let (svc, _ws) = build_service();
    // Missing required args → invalid_argument error, wrapped in the same envelope.
    let r = svc
        .dispatch("search_catalog", json!({ "source_id": "does-not-exist" }))
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap(), "expected failure: {r:?}");
    let keys = meta_keys(&r);
    // Error envelopes must carry the category for client-side dispatch and
    // all baseline counters (zero-valued, but present) so clients can parse
    // errors and success the same way.
    assert!(
        keys.contains("error_category"),
        "_meta missing error_category on error response: {keys:?}"
    );
    assert_has_baseline_keys(&keys, "search_catalog (error path)");
    assert_eq!(
        r["error"]["category"], r["_meta"]["error_category"],
        "category on error vs _meta diverged: {r:?}"
    );
}
