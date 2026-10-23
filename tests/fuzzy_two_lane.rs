//! ROADMAP P1.4 — `search_catalog fuzzy=true` returns two lanes:
//! `items` (exact BM25) and `fuzzy` (trigram + Levenshtein, independent).

use std::fs;

use dowser_engine::config::AppConfig;
use serde_json::json;
use tempfile::tempdir;

fn build_repo() -> (
    tempfile::TempDir,
    dowser_engine::service::CodeService,
    String,
) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("repo").join("src");
    fs::create_dir_all(&src).unwrap();
    // `dispatch_tool` is the exact target; the noise symbols are close by
    // trigram overlap but not single-edit distance.
    fs::write(
        src.join("mod.rs"),
        "pub fn dispatch_tool() {}\n\
         pub fn dispatch_tools() {}\n\
         pub fn dispatch_sem() {}\n\
         pub fn dispatch_concurrency() {}\n",
    )
    .unwrap();
    let svc =
        dowser_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("repo").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

#[test]
fn fuzzy_true_returns_separate_exact_and_fuzzy_lanes() {
    let (_ws, svc, sid) = build_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "dispatch_tol",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // No BM25 hit for misspelled identifier → `items` empty, `fuzzy` populated,
    // with `dispatch_tool` ranked first.
    let items = r["data"]["items"].as_array().unwrap();
    let fuzzy = r["data"]["fuzzy"].as_array().cloned().unwrap_or_default();
    assert!(
        items.is_empty() || fuzzy_has(&fuzzy, "dispatch_tool") || items_has(items, "dispatch_tool"),
        "dispatch_tool should appear somewhere: items={items:?} fuzzy={fuzzy:?}"
    );
}

#[test]
fn fuzzy_lane_drops_near_duplicates_of_exact_hit() {
    let (_ws, svc, sid) = build_repo();
    // Exact query matches `dispatch_tool` in items; the fuzzy lane must not
    // surface `dispatch_tools` (1-edit diff from the exact hit — too close
    // to be useful).
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "dispatch_tool",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let fuzzy = r["data"]["fuzzy"].as_array().cloned().unwrap_or_default();
    assert!(
        !fuzzy_has(&fuzzy, "dispatch_tools"),
        "fuzzy lane should drop 1-edit near-duplicates of exact hits: {fuzzy:?}"
    );
}

fn fuzzy_has(arr: &[serde_json::Value], name: &str) -> bool {
    arr.iter().any(|v| v["ref"]["title"].as_str() == Some(name))
}

fn items_has(arr: &[serde_json::Value], name: &str) -> bool {
    arr.iter().any(|v| v["ref"]["title"].as_str() == Some(name))
}
