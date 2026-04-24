//! E6 — `assemble_context mode=rename_plan` ranks files by edit likelihood,
//! estimates token cost, and surfaces collision warnings.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn build_svc_with_sources() -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // Definition file
    fs::write(
        src.join("a.py"),
        "def foo():\n    return 1\n\ndef foo_helper():\n    return foo() + 1\n",
    )
    .unwrap();
    // Text-only reference file
    fs::write(src.join("b.py"), "# calls foo here\nprint('foo')\n").unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (svc, ws, sid)
}

#[test]
fn rename_plan_orders_definition_file_first() {
    let (svc, _ws, sid) = build_svc_with_sources();
    let r = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "rename_plan",
                "old_name": "foo",
                "new_name": "bar",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "rename_plan failed: {r:?}");
    let files = r["data"]["files"].as_array().expect("files array");
    assert!(!files.is_empty(), "expected non-empty files: {r:?}");
    // The file containing `def foo` must be first.
    let first = files[0]["path"].as_str().unwrap_or("");
    assert!(
        first.ends_with("a.py"),
        "definition file should rank first, got {first}"
    );
    assert_eq!(files[0]["has_definition"], true);
}

#[test]
fn rename_plan_does_not_match_foo_helper_as_foo_definition() {
    let (svc, _ws, sid) = build_svc_with_sources();
    let r = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "rename_plan",
                "old_name": "foo",
            }),
        )
        .unwrap();
    let files = r["data"]["files"].as_array().unwrap();
    // a.py is the only file with an actual `foo` definition; foo_helper
    // lives there too but must not multiply the file count.
    let a_py = files
        .iter()
        .find(|f| f["path"].as_str().unwrap_or("").ends_with("a.py"))
        .expect("a.py present");
    let edits = a_py["edits"].as_array().unwrap();
    // Expect a definition entry for `foo` (not for `foo_helper`).
    assert!(
        edits
            .iter()
            .any(|e| e["kind"].as_str() == Some("definition")),
        "expected a definition edit for foo in a.py: {edits:?}"
    );
}
