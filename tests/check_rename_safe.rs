//! Slice tests for rename safety, plus explicit MCP error/meta checks.

use codesight_analysis::impact::rename_safety_payload;
use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_storage::storage::IndexStore;
use codesight_mcp::test_support::{
    find_symbol_id, index_code_fixture, open_code_store, write_files,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn check_rename_safe_suite() {}

fn build_repo() -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            (
                "utils.py",
                "def helper():\n    return 42\n\ndef other():\n    return 0\n",
            ),
            ("main.py", "from utils import helper\n\nresult = helper()\n"),
            (
                "cli.py",
                "from main import result\nfrom utils import other\nprint(result)\n",
            ),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

#[test]
fn test_safe_rename_returns_true() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "compute")
        .unwrap()
        .unwrap();
    assert!(r["safe"].as_bool().unwrap());
    assert!(r["conflicts"].as_array().unwrap().is_empty());
    assert!(r["checked_files"].as_u64().unwrap() >= 1);
}

#[test]
fn test_collision_detected() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "other")
        .unwrap()
        .unwrap();
    assert!(!r["safe"].as_bool().unwrap());
    let conflicts = r["conflicts"].as_array().unwrap();
    assert!(!conflicts.is_empty());
    let names: Vec<&str> = conflicts
        .iter()
        .filter_map(|c| c["existing_name"].as_str())
        .collect();
    assert!(names.contains(&"other"), "{names:?}");
}

#[test]
fn test_missing_symbol_returns_none_from_domain() {
    let (_ws, _config, _sid, store) = build_repo();
    assert!(
        rename_safety_payload(&store, "nonexistent_symbol_xyz", "foo")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": "no_such_repo",
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "x",
                    "new_name": "y",
                }],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error for missing repo: {r:?}"
    );
}

#[test]
fn test_result_includes_symbol_info() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "foo").unwrap().unwrap();
    assert_eq!(r["symbol"]["name"].as_str().unwrap(), "helper");
    assert_eq!(r["new_name"].as_str().unwrap(), "foo");
}

#[test]
fn test_timing_present() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            ("utils.py", "def helper():\n    return 42\n"),
            ("main.py", "from utils import helper\nresult = helper()\n"),
        ],
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap().to_string();
    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "helper",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    let sym = search["data"]["items"][0]["ref"]["id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": indexed["data"]["source_id"],
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": sym,
                    "new_name": "foo",
                }],
            }),
        )
        .unwrap();
    assert!(r.get("_meta").is_some(), "_meta missing: {r:?}");
    assert!(r["_meta"]["timing_ms"].is_number(), "timing_ms missing");
}
