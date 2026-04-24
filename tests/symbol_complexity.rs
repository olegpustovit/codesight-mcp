//! Slice tests for symbol complexity, plus a small MCP-envelope contract check.

use codesight_analysis::quality::complexity_payload;
use codesight_engine::config::AppConfig;
use codesight_mcp::test_support::{
    find_symbol_id, index_code_fixture, open_code_store, write_files,
};
use codesight_storage::storage::IndexStore;
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_repo() -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[(
            "funcs.py",
            concat!(
                "def simple():\n    return 1\n\n",
                "def branchy(x, y, z):\n",
                "    if x > 0:\n",
                "        if y > 0:\n",
                "            if z > 0:\n",
                "                return x + y + z\n",
                "            else:\n",
                "                return x + y\n",
                "        elif y < 0:\n",
                "            return x - y\n",
                "        else:\n",
                "            return x\n",
                "    elif x < 0:\n",
                "        return -x\n",
                "    else:\n",
                "        return 0\n",
            ),
        )],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

fn complexity(store: &IndexStore, symbol_id: &str) -> Value {
    complexity_payload(store, symbol_id).unwrap().unwrap()
}

#[test]
fn sym_complexity_returns_complexity_fields() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "branchy").unwrap();
    let r = complexity(&store, &sym);
    let ev = &r["findings"][0]["evidence"];
    assert!(ev["cyclomatic"].is_number(), "missing cyclomatic");
    assert!(ev["max_nesting"].is_number(), "missing max_nesting");
    assert!(ev["param_count"].is_number(), "missing param_count");
    assert!(
        r["findings"][0]["severity"].is_string() || r["findings"][0].get("assessment").is_some(),
        "missing assessment"
    );
}

#[test]
fn sym_complexity_assessment_values() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "simple").unwrap();
    let r = complexity(&store, &sym);
    let finding = &r["findings"][0];
    let assessment = finding
        .get("assessment")
        .or(finding.get("severity"))
        .unwrap()
        .as_str()
        .unwrap();
    assert!(["low", "medium", "high", "info"].contains(&assessment));
}

#[test]
fn sym_complexity_branchy_higher_than_simple() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym_simple = find_symbol_id(&store, "simple").unwrap();
    let sym_branchy = find_symbol_id(&store, "branchy").unwrap();
    let r_simple = complexity(&store, &sym_simple);
    let r_branchy = complexity(&store, &sym_branchy);

    let hints_simple = r_simple["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    let hints_branchy = r_branchy["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    assert!(hints_branchy >= hints_simple);
}

#[test]
fn sym_complexity_param_count_for_branchy() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "branchy").unwrap();
    let r = complexity(&store, &sym);
    assert_eq!(
        r["findings"][0]["evidence"]["param_count"]
            .as_u64()
            .unwrap(),
        3
    );
}

#[test]
fn sym_complexity_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": "x" }],
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn sym_complexity_missing_symbol_returns_none_from_domain() {
    let (_ws, _config, _sid, store) = build_repo();
    assert!(
        complexity_payload(&store, "nonexistent::symbol#function")
            .unwrap()
            .is_none()
    );
}

#[test]
fn sym_complexity_timing_present() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(&src, &[("simple.py", "def simple():\n    return 1\n")]).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap().to_string();
    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "simple",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    let sym = search["data"]["items"][0]["ref"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": indexed["data"]["source_id"],
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["_meta"]["timing_ms"].is_number());
}

#[test]
fn sym_complexity_repo_field_present() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "simple").unwrap();
    let r = complexity(&store, &sym);
    let repo = r["repo"].as_str().unwrap();
    assert!(repo.contains('/'));
}

#[test]
fn sym_complexity_file_and_line_fields() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "simple").unwrap();
    let r = complexity(&store, &sym);
    assert!(r["file"].is_string());
    assert!(r["line"].is_number());
}
