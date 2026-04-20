//! Tests for Workstream 3: response-level mitigation in `read_content`.
//!
//! Covers:
//! - Read scope classification (symbol, section, file_range, file_full)
//! - TOKEN_HEAVY_READ warning emission
//! - Extended next_step_hint for expensive reads
//! - `_meta.read_scopes` / `_meta.returned_bytes` / `_meta.warned_full_file_reads`

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn setup() -> (CodeService, String) {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(dir.path().join("idx"))).unwrap();

    let root = dir.path().join("test-repo");
    fs::create_dir_all(&root).unwrap();

    // Create a large file (above default 4096 threshold).
    let large_content = format!(
        "def big_function():\n    pass\n\ndef another():\n    pass\n\n{}",
        "# padding\n".repeat(500)
    );
    fs::write(root.join("large.py"), &large_content).unwrap();

    // Create a small file (below threshold).
    fs::write(root.join("small.py"), "def tiny():\n    return 1\n").unwrap();

    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();

    // Leak the tempdir so files persist for reads.
    std::mem::forget(dir);
    (svc, sid)
}

// ── Scope classification ─────────────────────────────────────────────

#[test]
fn symbol_read_has_symbol_scope() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "symbol", "path": "large.py", "id": "large.py::big_function#function"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "expected at least one item");
    assert_eq!(
        items[0]["read_scope"].as_str().unwrap_or(""),
        "symbol",
        "symbol read should have read_scope=symbol"
    );

    // Per-tool counters live under session_stats after the WS-2 envelope slim
    // (not on each per-call `_meta`). Verify the scope tally there.
    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert_eq!(detail["symbol_reads"].as_u64().unwrap(), 1);
    assert_eq!(detail["file_full_reads"].as_u64().unwrap(), 0);
}

#[test]
fn file_full_read_has_full_scope() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert_eq!(
        items[0]["read_scope"].as_str().unwrap_or(""),
        "file_full",
        "whole-file read should have read_scope=file_full"
    );

    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert_eq!(detail["file_full_reads"].as_u64().unwrap(), 1);
    assert_eq!(detail["symbol_reads"].as_u64().unwrap(), 0);
}

#[test]
fn file_range_read_has_range_scope() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py", "start_line": 1, "end_line": 5}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert_eq!(
        items[0]["read_scope"].as_str().unwrap_or(""),
        "file_range",
        "bounded read should have read_scope=file_range"
    );

    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert_eq!(detail["file_range_reads"].as_u64().unwrap(), 1);
    assert_eq!(detail["file_full_reads"].as_u64().unwrap(), 0);
}

// ── TOKEN_HEAVY_READ warning ─────────────────────────────────────────

#[test]
fn heavy_full_file_read_emits_warning() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");

    let warnings = r["warnings"].as_array().unwrap();
    let heavy: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"].as_str() == Some("TOKEN_HEAVY_READ"))
        .collect();
    assert!(
        !heavy.is_empty(),
        "expected TOKEN_HEAVY_READ warning for large whole-file read, warnings: {warnings:?}"
    );
    assert!(
        heavy[0]["detail"]["returned_bytes"].as_u64().unwrap() > 4096,
        "returned_bytes should exceed threshold"
    );

    // session_stats now tracks aggregate warning / byte counts across calls.
    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert!(
        detail["warning_count"].as_u64().unwrap() > 0,
        "warning_count should be > 0 after a TOKEN_HEAVY_READ: {detail}"
    );
}

#[test]
fn small_full_file_read_no_warning() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "small.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");

    let warnings = r["warnings"].as_array().unwrap();
    let heavy: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"].as_str() == Some("TOKEN_HEAVY_READ"))
        .collect();
    assert!(
        heavy.is_empty(),
        "small file should not trigger TOKEN_HEAVY_READ: {warnings:?}"
    );
    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert_eq!(detail["warning_count"].as_u64().unwrap(), 0);
}

#[test]
fn bounded_read_no_warning() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py", "start_line": 1, "end_line": 5}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");

    let warnings = r["warnings"].as_array().unwrap();
    let heavy: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"].as_str() == Some("TOKEN_HEAVY_READ"))
        .collect();
    assert!(
        heavy.is_empty(),
        "bounded read should not trigger TOKEN_HEAVY_READ"
    );
}

#[test]
fn symbol_read_no_warning() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "symbol", "path": "large.py", "id": "large.py::big_function#function"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r}");

    let warnings = r["warnings"].as_array().unwrap();
    let heavy: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"].as_str() == Some("TOKEN_HEAVY_READ"))
        .collect();
    assert!(
        heavy.is_empty(),
        "symbol read should never trigger TOKEN_HEAVY_READ"
    );
}

// ── next_step_hint ───────────────────────────────────────────────────

#[test]
fn heavy_plain_read_has_alternatives_hint() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());

    let hint = &r["_meta"]["next_step_hint"];
    assert!(
        hint.is_object(),
        "heavy read should have next_step_hint: {r}"
    );
    assert_eq!(
        hint["tool"].as_str().unwrap(),
        "inspect_structure",
        "hint should suggest inspect_structure"
    );
    assert!(
        hint["alternatives"].is_array(),
        "hint should include alternatives array"
    );
}

#[test]
fn verify_read_has_edit_safety_hint_not_alternatives() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py"}],
                "verify": true
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());

    let hint = &r["_meta"]["next_step_hint"];
    assert!(hint.is_object());
    // Verify/hash hint should mention edit-safety, not alternatives.
    assert_eq!(hint["tool"].as_str().unwrap(), "read_content");
    assert!(
        hint.get("alternatives").is_none(),
        "verify hint should not have alternatives"
    );
}

#[test]
fn cheap_plain_read_no_hint() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "small.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    assert!(
        r["_meta"].get("next_step_hint").is_none(),
        "cheap read should not have next_step_hint"
    );
}

// ── _meta counters ───────────────────────────────────────────────────

#[test]
fn meta_returned_bytes_is_positive() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{"kind": "file", "path": "large.py"}]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert!(
        detail["bytes_returned"].as_u64().unwrap() > 0,
        "bytes_returned should be positive: {detail}"
    );
}

#[test]
fn mixed_refs_have_correct_scope_counts() {
    let (svc, sid) = setup();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [
                    {"kind": "symbol", "path": "large.py", "id": "large.py::big_function#function"},
                    {"kind": "file", "path": "large.py"},
                    {"kind": "file", "path": "large.py", "start_line": 1, "end_line": 3}
                ]
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());

    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    let detail = &stats["data"]["per_tool"]["read_content"]["read_content_detail"];
    assert_eq!(detail["symbol_reads"].as_u64().unwrap(), 1);
    assert_eq!(detail["file_full_reads"].as_u64().unwrap(), 1);
    assert_eq!(detail["file_range_reads"].as_u64().unwrap(), 1);
}
