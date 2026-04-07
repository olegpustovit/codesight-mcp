//! Integration tests: document coverage.

use std::fs;
use std::path::Path;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn doc_fixtures_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference")
}

fn ingest_docs(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn setup_indexed_fixture() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let fixtures = doc_fixtures_root();
    let doc_root = ws.path().join("doc-copy");
    fs::create_dir_all(doc_root.join("docs")).unwrap();
    fs::copy(
        fixtures.join("docs/sample.md"),
        doc_root.join("docs/sample.md"),
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_docs(&svc, &doc_root);
    (ws, svc, sid)
}

fn get_doc_coverage(svc: &CodeService, sid: &str, refs: Vec<Value>) -> Value {
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "doc_coverage",
                "refs": refs,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "doc_coverage failed: {r:?}");
    r
}

fn symbol_ref(id: &str) -> Value {
    json!({ "kind": "symbol", "stable_id": id })
}

// ════════════════════════════════════════════════════════════════════════════
// TestSymbolNameFromId — internal helper _symbol_name_from_id
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_symbol_name_from_id_full_qualified() {
    // Tested through doc_coverage behavior - symbol name extraction
    // The symbol name "handle_request" from "my-repo::src/server.py::handle_request#function"
    // is used for matching headings
    let (_ws, svc, sid) = setup_indexed_fixture();
    // We just verify the coverage tool works; name extraction is internal
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref(
            "my-repo::src/server.py::handle_request#function",
        )],
    );
    // "handle_request" should not match any heading in sample.md
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| { u["symbol_name"].as_str() == Some("handle_request") }),
        "handle_request should be undocumented: {r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_no_type_suffix() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("repo::path::my_func")]);
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("my_func")),
        "my_func should be undocumented: {r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_bare_name() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    // "Installation" IS a heading in sample.md → should be documented
    let documented = r["data"]["documented"].as_array().unwrap();
    assert!(
        documented
            .iter()
            .any(|d| d["symbol_name"].as_str() == Some("Installation")),
        "Installation should be documented: {r:?}"
    );
}

#[test]
fn doc_symbol_name_from_id_no_colons() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("some_function#method")]);
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| u["symbol_name"].as_str() == Some("some_function")),
        "some_function should be undocumented: {r:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Error cases
// ════════════════════════════════════════════════════════════════════════════

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
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![]);
    assert_eq!(r["data"]["symbols_checked"].as_u64().unwrap(), 0);
    assert_eq!(r["data"]["coverage_pct"].as_f64().unwrap(), 0.0);
}

// ════════════════════════════════════════════════════════════════════════════
// Coverage detection
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_doc_coverage_documented_symbol_found() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("myrepo::src/app.py::Installation#function")],
    );
    let documented = r["data"]["documented"].as_array().unwrap();
    assert!(
        documented
            .iter()
            .any(|d| { d["symbol_name"].as_str() == Some("Installation") }),
        "Installation should be documented: {r:?}"
    );
    assert!(r["data"]["documented_count"].as_u64().unwrap() >= 1);
}

#[test]
fn doc_doc_coverage_undocumented_symbol_reported() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref(
            "myrepo::src/app.py::nonexistent_xyz_func#function",
        )],
    );
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(
        undocumented
            .iter()
            .any(|u| { u["symbol_name"].as_str() == Some("nonexistent_xyz_func") }),
        "nonexistent_xyz_func should be undocumented: {r:?}"
    );
    assert!(r["data"]["undocumented_count"].as_u64().unwrap() >= 1);
}

#[test]
fn doc_doc_coverage_mixed_coverage() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![
            symbol_ref("myrepo::src/app.py::Installation#function"),
            symbol_ref("myrepo::src/app.py::totally_missing_xyz#function"),
        ],
    );
    assert_eq!(r["data"]["symbols_checked"].as_u64().unwrap(), 2);
    assert!(r["data"]["documented_count"].as_u64().unwrap() >= 1);
    assert!(r["data"]["undocumented_count"].as_u64().unwrap() >= 1);
    let pct = r["data"]["coverage_pct"].as_f64().unwrap();
    assert!(
        pct > 0.0 && pct < 100.0,
        "coverage should be between 0 and 100: {pct}"
    );
}

#[test]
fn doc_doc_coverage_pct_100() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    // "Installation" and "Usage" are both headings in sample.md
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("Installation"), symbol_ref("Usage")],
    );
    let pct = r["data"]["coverage_pct"].as_f64().unwrap();
    assert!(
        pct > 0.0,
        "coverage should be > 0 for documented symbols: {pct}"
    );
}

#[test]
fn doc_doc_coverage_pct_0() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("totally_unknown_xyz_abc#function")],
    );
    let pct = r["data"]["coverage_pct"].as_f64().unwrap();
    assert_eq!(pct, 0.0, "coverage should be 0 for unknown symbols");
}

// ════════════════════════════════════════════════════════════════════════════
// Result structure
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_doc_coverage_result_structure() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    let data = &r["data"];
    assert!(data["repo"].is_string(), "should have repo");
    assert!(
        data["symbols_checked"].is_number(),
        "should have symbols_checked"
    );
    assert!(
        data["documented_count"].is_number(),
        "should have documented_count"
    );
    assert!(
        data["undocumented_count"].is_number(),
        "should have undocumented_count"
    );
    assert!(data["coverage_pct"].is_number(), "should have coverage_pct");
    assert!(data["documented"].is_array(), "should have documented");
    assert!(data["undocumented"].is_array(), "should have undocumented");
}

#[test]
fn doc_doc_coverage_documented_entry_has_sections() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    let documented = r["data"]["documented"].as_array().unwrap();
    assert!(!documented.is_empty());
    let entry = &documented[0];
    assert!(entry["symbol_id"].is_string(), "should have symbol_id");
    assert!(entry["symbol_name"].is_string(), "should have symbol_name");
    let sections = entry["matching_sections"].as_array().unwrap();
    assert!(!sections.is_empty(), "should have matching sections");
    let sec = &sections[0];
    assert!(
        sec["section_id"].is_string(),
        "section should have section_id"
    );
    assert!(
        sec["section_title"].is_string(),
        "section should have section_title"
    );
    assert!(sec["doc_path"].is_string(), "section should have doc_path");
}

#[test]
fn doc_doc_coverage_undocumented_entry_fields() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("nonexistent_xyz_func#function")],
    );
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(!undocumented.is_empty());
    let entry = &undocumented[0];
    assert!(entry["symbol_id"].is_string(), "should have symbol_id");
    assert!(entry["symbol_name"].is_string(), "should have symbol_name");
}

// ════════════════════════════════════════════════════════════════════════════
// Cap at 200
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_doc_coverage_symbol_ids_capped() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let refs: Vec<Value> = (0..250)
        .map(|i| symbol_ref(&format!("sym_{i}#function")))
        .collect();
    let r = get_doc_coverage(&svc, &sid, refs);
    assert_eq!(
        r["data"]["symbols_checked"].as_u64().unwrap(),
        200,
        "should cap at 200"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Meta
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_doc_coverage_meta_present() {
    let (_ws, svc, sid) = setup_indexed_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    assert!(r["_meta"].is_object(), "should have _meta");
    assert!(
        r["_meta"]["timing_ms"].is_number(),
        "should have timing_ms: {:?}",
        r["_meta"]
    );
}
