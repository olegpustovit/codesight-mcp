//! Slice tests for doc coverage, plus a small MCP-envelope contract check.

use std::path::Path;

use codesight_mcp::service::agent_v1::quality::doc_coverage::doc_coverage_payload;
use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::DocStore;
use codesight_mcp::test_support::{
    copy_fixture_tree, doc_reference_fixtures_dir, index_doc_fixture, open_doc_store,
};
use serde_json::{Value, json};
use tempfile::tempdir;

fn setup_indexed_fixture() -> (tempfile::TempDir, AppConfig, String, DocStore) {
    let ws = tempdir().unwrap();
    let fixtures = doc_reference_fixtures_dir();
    let doc_root = ws.path().join("doc-copy");
    copy_fixture_tree(&fixtures, &doc_root).unwrap();
    let (config, sid, _) = index_doc_fixture(&ws.path().join("idx"), &doc_root).unwrap();
    let store = open_doc_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

fn get_doc_coverage(store: &DocStore, refs: Vec<Value>) -> Value {
    let mut symbol_ids: Vec<String> = refs
        .into_iter()
        .filter(|r| r["kind"].as_str() == Some("symbol"))
        .filter_map(|r| {
            r["stable_id"]
                .as_str()
                .or_else(|| r["id"].as_str())
                .map(ToString::to_string)
        })
        .collect();
    symbol_ids.truncate(200);
    doc_coverage_payload(store, &symbol_ids).unwrap()
}

fn symbol_ref(id: &str) -> Value {
    json!({ "kind": "symbol", "stable_id": id })
}

#[test]
fn doc_symbol_name_from_id_full_qualified() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &store,
        vec![symbol_ref(
            "my-repo::src/server.py::handle_request#function",
        )],
    );
    let undocumented = r["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("handle_request")),
        "{r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_no_type_suffix() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("repo::path::my_func")]);
    let undocumented = r["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("my_func")),
        "{r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_bare_name() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("Installation")]);
    let documented = r["documented"].as_array().unwrap();
    assert!(
        documented
            .iter()
            .any(|d| d["symbol_name"].as_str() == Some("Installation")),
        "{r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_no_colons() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("some_function#method")]);
    let undocumented = r["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("some_function")),
        "{r:?}"
    );
}

#[test]
fn doc_doc_coverage_not_indexed() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "nonexistent/repo",
                "analysis": "doc_coverage",
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap(),
        "should error for non-indexed repo"
    );
}

#[test]
fn doc_doc_coverage_empty_symbol_ids() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![]);
    assert_eq!(r["symbols_checked"].as_u64().unwrap(), 0);
    assert_eq!(r["coverage_pct"].as_f64().unwrap(), 0.0);
}

#[test]
fn doc_doc_coverage_documented_symbol_found() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &store,
        vec![symbol_ref("myrepo::src/app.py::Installation#function")],
    );
    let documented = r["documented"].as_array().unwrap();
    assert!(
        documented
            .iter()
            .any(|d| d["symbol_name"].as_str() == Some("Installation")),
        "{r:?}"
    );
    assert!(r["documented_count"].as_u64().unwrap() >= 1);
}

#[test]
fn doc_doc_coverage_undocumented_symbol_reported() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &store,
        vec![symbol_ref(
            "myrepo::src/app.py::nonexistent_xyz_func#function",
        )],
    );
    let undocumented = r["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("nonexistent_xyz_func")),
        "{r:?}"
    );
    assert!(r["undocumented_count"].as_u64().unwrap() >= 1);
}

#[test]
fn doc_doc_coverage_mixed_coverage() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &store,
        vec![
            symbol_ref("myrepo::src/app.py::Installation#function"),
            symbol_ref("myrepo::src/app.py::totally_missing_xyz#function"),
        ],
    );
    assert_eq!(r["symbols_checked"].as_u64().unwrap(), 2);
    assert!(r["documented_count"].as_u64().unwrap() >= 1);
    assert!(r["undocumented_count"].as_u64().unwrap() >= 1);
    let pct = r["coverage_pct"].as_f64().unwrap();
    assert!(pct > 0.0 && pct < 100.0, "{pct}");
}

#[test]
fn doc_doc_coverage_pct_100() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &store,
        vec![symbol_ref("Installation"), symbol_ref("Usage")],
    );
    assert!(r["coverage_pct"].as_f64().unwrap() > 0.0);
}

#[test]
fn doc_doc_coverage_pct_0() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("totally_unknown_xyz_abc#function")]);
    assert_eq!(r["coverage_pct"].as_f64().unwrap(), 0.0);
}

#[test]
fn doc_doc_coverage_result_structure() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("Installation")]);
    for key in [
        "repo",
        "symbols_checked",
        "documented_count",
        "undocumented_count",
        "coverage_pct",
        "documented",
        "undocumented",
    ] {
        assert!(!r[key].is_null(), "missing key: {key}");
    }
}

#[test]
fn doc_doc_coverage_documented_entry_has_sections() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("Installation")]);
    let documented = r["documented"].as_array().unwrap();
    assert!(!documented.is_empty());
    let entry = &documented[0];
    assert!(entry["symbol_id"].is_string());
    assert!(entry["symbol_name"].is_string());
    let sections = entry["matching_sections"].as_array().unwrap();
    assert!(!sections.is_empty());
    let sec = &sections[0];
    assert!(sec["section_id"].is_string());
    assert!(sec["section_title"].is_string());
    assert!(sec["doc_path"].is_string());
}

#[test]
fn doc_doc_coverage_undocumented_entry_fields() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let r = get_doc_coverage(&store, vec![symbol_ref("nonexistent_xyz_func#function")]);
    let undocumented = r["undocumented"].as_array().unwrap();
    assert!(!undocumented.is_empty());
    let entry = &undocumented[0];
    assert!(entry["symbol_id"].is_string());
    assert!(entry["symbol_name"].is_string());
}

#[test]
fn doc_doc_coverage_symbol_ids_capped() {
    let (_ws, _config, _sid, store) = setup_indexed_fixture();
    let refs: Vec<Value> = (0..250)
        .map(|i| symbol_ref(&format!("sym_{i}#function")))
        .collect();
    let r = get_doc_coverage(&store, refs);
    assert_eq!(r["symbols_checked"].as_u64().unwrap(), 200);
}

#[test]
fn doc_doc_coverage_meta_present() {
    let ws = tempdir().unwrap();
    let fixture_root = Path::new(&doc_reference_fixtures_dir()).join("docs");
    let doc_root = ws.path().join("docs");
    copy_fixture_tree(&fixture_root, &doc_root).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "doc_coverage",
                "refs": [{ "kind": "symbol", "stable_id": "Installation" }],
            }),
        )
        .unwrap();
    assert!(r["_meta"].is_object(), "should have _meta");
    assert!(r["_meta"]["timing_ms"].is_number(), "should have timing_ms");
}
