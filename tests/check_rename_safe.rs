//! Integration tests: check rename safe.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn check_rename_safe_suite() {
    // Master compilation test; individual tests below run independently.
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn build_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("utils.py"),
        "def helper():\n    return 42\n\ndef other():\n    return 0\n",
    )
    .unwrap();
    fs::write(
        src.join("main.py"),
        "from utils import helper\n\nresult = helper()\n",
    )
    .unwrap();
    fs::write(
        src.join("cli.py"),
        "from main import result\nfrom utils import other\nprint(result)\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

fn find_symbol_id(svc: &CodeService, source_id: &str, name: &str) -> String {
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": name,
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 20,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    for it in res["data"]["items"].as_array().unwrap() {
        if it["ref"]["title"].as_str() == Some(name) {
            return it["ref"]["id"].as_str().unwrap().to_string();
        }
    }
    panic!("symbol {name:?} not found in search results");
}

// ── TestCheckRenameSafe ──────────────────────────────────────────────────────

#[test]
fn test_safe_rename_returns_true() {
    // rename "helper" to "compute" → safe=true, conflicts=[], checked_files >= 1
    let (_ws, svc, sid) = build_repo();
    let sym = find_symbol_id(&svc, &sid, "helper");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": sym,
                    "new_name": "compute",
                }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let d = &r["data"];
    assert!(d["safe"].as_bool().unwrap());
    assert!(d["conflicts"].as_array().unwrap().is_empty());
    assert!(d["checked_files"].as_u64().unwrap() >= 1);
}

#[test]
fn test_collision_detected() {
    // rename "helper" to "other" → safe=false, conflicts with "other"
    let (_ws, svc, sid) = build_repo();
    let sym = find_symbol_id(&svc, &sid, "helper");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": sym,
                    "new_name": "other",
                }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let d = &r["data"];
    assert!(!d["safe"].as_bool().unwrap());
    let conflicts = d["conflicts"].as_array().unwrap();
    assert!(!conflicts.is_empty());
    let names: Vec<&str> = conflicts
        .iter()
        .filter_map(|c| c["existing_name"].as_str())
        .collect();
    assert!(
        names.contains(&"other"),
        "expected 'other' in conflicts: {names:?}"
    );
}

#[test]
fn test_missing_symbol_returns_error() {
    let (_ws, svc, sid) = build_repo();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "nonexistent_symbol_xyz",
                    "new_name": "foo",
                }],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error for missing symbol: {r:?}"
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
    // result["symbol"]["name"] == "helper", result["new_name"] == "foo"
    let (_ws, svc, sid) = build_repo();
    let sym = find_symbol_id(&svc, &sid, "helper");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": sym,
                    "new_name": "foo",
                }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let d = &r["data"];
    assert_eq!(d["symbol"]["name"].as_str().unwrap(), "helper");
    assert_eq!(d["new_name"].as_str().unwrap(), "foo");
}

#[test]
fn test_timing_present() {
    // _meta has timing_ms
    let (_ws, svc, sid) = build_repo();
    let sym = find_symbol_id(&svc, &sid, "helper");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
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
