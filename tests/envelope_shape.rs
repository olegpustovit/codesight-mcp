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
    "tokens_saved_per_ms",
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

fn assert_tokens_saved_per_ms_sane(response: &Value, tool: &str) {
    let v = response["_meta"]["tokens_saved_per_ms"]
        .as_f64()
        .unwrap_or_else(|| panic!("{tool}: tokens_saved_per_ms not a number"));
    assert!(v.is_finite(), "{tool}: tokens_saved_per_ms must be finite");
    assert!(v >= 0.0, "{tool}: tokens_saved_per_ms must be non-negative");
}

fn assert_cost_maps_rounded(response: &Value, tool: &str) {
    for key in ["cost_avoided", "total_cost_avoided"] {
        let map = response["_meta"][key]
            .as_object()
            .unwrap_or_else(|| panic!("{tool}: {key} should be an object"));
        for (model, value) in map {
            let v = value
                .as_f64()
                .unwrap_or_else(|| panic!("{tool}: {key}.{model} should be a number"));
            let scaled = v * 10_000.0;
            assert!(
                (scaled - scaled.round()).abs() < 1e-9,
                "{tool}: {key}.{model} should be rounded to 4 decimals, got {v}"
            );
        }
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
    assert_tokens_saved_per_ms_sane(&r, "search_catalog");
    assert_cost_maps_rounded(&r, "search_catalog");
    let kinds = r["_meta"]["unit_kinds_effective"]
        .as_object()
        .expect("search_catalog: missing unit_kinds_effective object");
    assert_eq!(kinds.get("symbol").and_then(Value::as_bool), Some(true));
    assert_eq!(kinds.get("section").and_then(Value::as_bool), Some(false));
    assert_eq!(kinds.get("file").and_then(Value::as_bool), Some(false));
    assert!(
        r["_meta"]["fuzzy_applied"].is_boolean(),
        "search_catalog: missing fuzzy_applied bool: {}",
        r["_meta"]
    );
    assert_eq!(
        r["_meta"]["estimate_method"].as_str(),
        Some("byte_approx_candidate_files"),
        "search_catalog: estimate_method drifted: {}",
        r["_meta"]
    );
    assert_eq!(
        r["_meta"]["baseline_scope"].as_str(),
        Some("candidate_files_materialized_pre_output"),
        "search_catalog: baseline_scope drifted: {}",
        r["_meta"]
    );
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
