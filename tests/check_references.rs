//! Integration tests: check references.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn check_references_suite() {
    run_all();
}

fn run_all() {
    test_finds_import_reference();
    test_finds_content_reference();
    test_no_references();
    test_search_content_false();
    test_batch_identifiers();
    test_not_indexed();
    test_no_import_data_searches_content();
    test_neither_param_raises();
    test_both_params_raises();
}

// ── helpers ──────────────────────────────────────────────────────────────────

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
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

// ── test_finds_import_reference ──────────────────────────────────────────────
// index JS files with import, check_references → is_referenced=true, import_count>=1

fn test_finds_import_reference() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.js"), "export function helper() {}\n").unwrap();
    fs::write(src.join("app.js"), "import { helper } from './utils';\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data["is_referenced"].as_bool().unwrap(),
        "expected helper to be referenced: {data:?}"
    );
    assert!(
        data["import_count"].as_u64().unwrap() >= 1
            || data["content_count"].as_u64().unwrap_or(0) >= 1,
        "expected import_count >= 1 or content_count >= 1: {data:?}"
    );
    assert!(
        data["evidence_notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["signal"] == "import_edges" && note["source"] == "structural"),
        "expected structural import evidence notes: {data:?}"
    );
}

// ── test_finds_content_reference ─────────────────────────────────────────────

fn test_finds_content_reference() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();
    fs::write(
        src.join("app.py"),
        "from utils import helper\nresult = helper()\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data["is_referenced"].as_bool().unwrap(),
        "expected helper to be referenced: {data:?}"
    );
    // Content or import should detect it
    let total =
        data["import_count"].as_u64().unwrap_or(0) + data["content_count"].as_u64().unwrap_or(0);
    assert!(total >= 1, "expected at least 1 reference: {data:?}");
}

// ── test_no_references ───────────────────────────────────────────────────────

fn test_no_references() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();
    fs::write(src.join("app.py"), "print('hello')\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        !data["is_referenced"].as_bool().unwrap(),
        "expected helper NOT to be referenced: {data:?}"
    );
    assert_eq!(data["import_count"].as_u64().unwrap(), 0);
    assert_eq!(data["content_count"].as_u64().unwrap(), 0);
}

// ── test_search_content_false ────────────────────────────────────────────────
// search_content=False skips content, only checks imports.

fn test_search_content_false() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();
    fs::write(
        src.join("app.py"),
        "# helper is used here in a comment\nprint('hello')\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
                "search_content": false,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    // With search_content=false, should not find the comment reference
    assert!(
        !data["is_referenced"].as_bool().unwrap(),
        "expected no references with search_content=false: {data:?}"
    );
    // content_references should not be present
    assert!(
        data.get("content_references").is_none(),
        "content_references should not appear when search_content=false: {data:?}"
    );
}

// ── test_batch_identifiers ───────────────────────────────────────────────────
// check_references with identifiers list returns grouped results.

fn test_batch_identifiers() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("utils.js"),
        "export function helper() {}\nexport function unused() {}\n",
    )
    .unwrap();
    fs::write(src.join("app.js"), "import { helper } from './utils';\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifiers": ["helper", "unused"],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    let results = data["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "expected 2 results for batch: {data:?}");

    // Find by identifier name
    let helper_r = results
        .iter()
        .find(|r| r["identifier"] == "helper")
        .unwrap();
    let unused_r = results
        .iter()
        .find(|r| r["identifier"] == "unused")
        .unwrap();
    assert!(
        helper_r["is_referenced"].as_bool().unwrap(),
        "helper should be referenced: {helper_r:?}"
    );
    assert!(
        !unused_r["is_referenced"].as_bool().unwrap(),
        "unused should NOT be referenced: {unused_r:?}"
    );
}

// ── test_not_indexed ─────────────────────────────────────────────────────────

fn test_not_indexed() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": "nonexistent/repo",
                "identifier": "foo",
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error for nonexistent repo: {r:?}"
    );
}

// ── test_no_import_data_searches_content ─────────────────────────────────────
// check_references should find content references even without explicit import data.

fn test_no_import_data_searches_content() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();
    fs::write(src.join("app.py"), "result = helper()\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    // Should find content reference in app.py
    assert!(
        data["is_referenced"].as_bool().unwrap(),
        "expected helper to be referenced via content: {data:?}"
    );
    assert!(
        data["content_count"].as_u64().unwrap() >= 1,
        "expected content_count >= 1: {data:?}"
    );
    assert!(
        data["evidence_breakdown"]["fallback_content_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file.as_str() == Some("src/app.py")),
        "expected src/app.py in fallback content evidence: {data:?}"
    );
    assert!(
        data["evidence_notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["signal"] == "fallback_file_scan" && note["source"] == "heuristic"),
        "expected fallback content evidence note: {data:?}"
    );
}

// ── test_neither_param_raises ────────────────────────────────────────────────

fn test_neither_param_raises() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error when neither identifier nor identifiers given: {r:?}"
    );
}

// ── test_both_params_raises ──────────────────────────────────────────────────

fn test_both_params_raises() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper(): pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "check_references",
            json!({
                "source_id": sid,
                "identifier": "helper",
                "identifiers": ["helper"],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error when both identifier and identifiers given: {r:?}"
    );
}
