//! Integration tests: get doc coverage.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

#[test]
fn get_doc_coverage_suite() {
    run_all();
}

fn run_all() {
    // _symbol_name_from_id helper (4 tests)
    test_symbol_name_from_id_full_qualified();
    test_symbol_name_from_id_no_type_suffix();
    test_symbol_name_from_id_bare_name();
    test_symbol_name_from_id_no_colons();

    // Error cases (2)
    test_not_indexed();
    test_empty_symbol_ids();

    // Coverage detection (5)
    test_documented_symbol_found();
    test_undocumented_symbol_reported();
    test_mixed_coverage();
    test_coverage_pct_100();
    test_coverage_pct_0();

    // Result structure (3)
    test_result_structure();
    test_documented_entry_has_sections();
    test_undocumented_entry_fields();

    // Cap at 200 (1)
    test_symbol_ids_capped();

    // Meta (1)
    test_meta_present();
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn ingest_docs(svc: &CodeService, root: &std::path::Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "docs",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "doc ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn make_doc_fixture() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let docs = ws.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        docs.join("sample.md"),
        "# Installation\n\nHow to install.\n\n## Usage\n\nHow to use.\n\n## API Reference\n\nAPI docs.\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_docs(&svc, ws.path());
    (ws, svc, sid)
}

fn symbol_ref(id: &str) -> Value {
    json!({ "kind": "symbol", "stable_id": id })
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

// ── _symbol_name_from_id helper tests ────────────────────────────────────────

fn extract_symbol_name(id: &str) -> &str {
    let last = id.rsplit("::").next().unwrap_or(id);
    match last.rfind('#') {
        Some(pos) => &last[..pos],
        None => last,
    }
}

fn test_symbol_name_from_id_full_qualified() {
    assert_eq!(
        extract_symbol_name("my-repo::src/server.py::handle_request#function"),
        "handle_request"
    );
}

fn test_symbol_name_from_id_no_type_suffix() {
    assert_eq!(extract_symbol_name("repo::path::my_func"), "my_func");
}

fn test_symbol_name_from_id_bare_name() {
    assert_eq!(extract_symbol_name("Installation"), "Installation");
}

fn test_symbol_name_from_id_no_colons() {
    assert_eq!(extract_symbol_name("some_function#method"), "some_function");
}

// ── Error cases ──────────────────────────────────────────────────────────────

fn test_not_indexed() {
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

fn test_empty_symbol_ids() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "doc_coverage",
                "refs": [],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap(),
        "empty refs must error, not silently return 0% (see CODESIGHT_TESTDRIVE_ISSUES_v3.md #14)"
    );
    assert_eq!(r["error"]["category"].as_str().unwrap(), "invalid_argument");
}

// ── Coverage detection ───────────────────────────────────────────────────────

fn test_documented_symbol_found() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("myrepo::src/app.py::Installation#function")],
    );
    let documented = r["data"]["documented"].as_array().unwrap();
    let names: Vec<&str> = documented
        .iter()
        .filter_map(|d| d["symbol_name"].as_str())
        .collect();
    assert!(
        names.contains(&"Installation"),
        "Installation should be documented: {names:?}"
    );
    assert!(r["data"]["documented_count"].as_u64().unwrap() >= 1);
}

fn test_undocumented_symbol_reported() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref(
            "myrepo::src/app.py::nonexistent_xyz_func#function",
        )],
    );
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    let names: Vec<&str> = undocumented
        .iter()
        .filter_map(|u| u["symbol_name"].as_str())
        .collect();
    assert!(
        names.contains(&"nonexistent_xyz_func"),
        "should be undocumented: {names:?}"
    );
    assert!(r["data"]["undocumented_count"].as_u64().unwrap() >= 1);
}

fn test_mixed_coverage() {
    let (_ws, svc, sid) = make_doc_fixture();
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
        "coverage should be partial: {pct}"
    );
}

fn test_coverage_pct_100() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("Installation"), symbol_ref("Usage")],
    );
    let pct = r["data"]["coverage_pct"].as_f64().unwrap();
    assert!(
        pct > 0.0,
        "documented symbols should yield coverage > 0: {pct}"
    );
}

fn test_coverage_pct_0() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("totally_unknown_xyz_abc#function")],
    );
    assert_eq!(r["data"]["coverage_pct"].as_f64().unwrap(), 0.0);
}

// ── Result structure ─────────────────────────────────────────────────────────

fn test_result_structure() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    let data = &r["data"];
    assert!(data["repo"].is_string());
    assert!(data["symbols_checked"].is_number());
    assert!(data["documented_count"].is_number());
    assert!(data["undocumented_count"].is_number());
    assert!(data["coverage_pct"].is_number());
    assert!(data["documented"].is_array());
    assert!(data["undocumented"].is_array());
}

fn test_documented_entry_has_sections() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    let documented = r["data"]["documented"].as_array().unwrap();
    assert!(!documented.is_empty());
    let entry = &documented[0];
    assert!(entry["symbol_id"].is_string());
    assert!(entry["symbol_name"].is_string());
    let sections = entry["matching_sections"].as_array().unwrap();
    assert!(!sections.is_empty());
    assert!(sections[0]["section_id"].is_string());
    assert!(sections[0]["section_title"].is_string());
    assert!(sections[0]["doc_path"].is_string());
}

fn test_undocumented_entry_fields() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(
        &svc,
        &sid,
        vec![symbol_ref("nonexistent_xyz_func#function")],
    );
    let undocumented = r["data"]["undocumented"].as_array().unwrap();
    assert!(!undocumented.is_empty());
    assert!(undocumented[0]["symbol_id"].is_string());
    assert!(undocumented[0]["symbol_name"].is_string());
}

// ── Cap at 200 ───────────────────────────────────────────────────────────────

fn test_symbol_ids_capped() {
    let (_ws, svc, sid) = make_doc_fixture();
    let refs: Vec<Value> = (0..250)
        .map(|i| symbol_ref(&format!("sym_{i}#function")))
        .collect();
    let r = get_doc_coverage(&svc, &sid, refs);
    assert_eq!(r["data"]["symbols_checked"].as_u64().unwrap(), 200);
}

// ── Meta ─────────────────────────────────────────────────────────────────────

fn test_meta_present() {
    let (_ws, svc, sid) = make_doc_fixture();
    let r = get_doc_coverage(&svc, &sid, vec![symbol_ref("Installation")]);
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}
