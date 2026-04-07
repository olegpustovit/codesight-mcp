//! Integration tests: dead code v2.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn build_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // Entry point
    fs::write(
        src.join("main.py"),
        "from utils import used_fn\n\nif __name__ == '__main__':\n    used_fn()\n",
    )
    .unwrap();
    // Module with one used and one dead function
    fs::write(
        src.join("utils.py"),
        "def used_fn():\n    return 1\n\ndef dead_fn():\n    return 2\n",
    )
    .unwrap();
    // Completely unreachable module
    fs::write(src.join("orphan.py"), "def orphan_fn():\n    pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn dead_code_v2(svc: &CodeService, sid: &str, min_confidence: f64) -> Value {
    svc.dispatch(
        "evaluate_quality",
        json!({
            "source_id": sid,
            "analysis": "dead_code_v2",
            "min_confidence": min_confidence,
        }),
    )
    .unwrap()
}

fn dead_code_v2_default(svc: &CodeService, sid: &str) -> Value {
    svc.dispatch(
        "evaluate_quality",
        json!({
            "source_id": sid,
            "analysis": "dead_code_v2",
        }),
    )
    .unwrap()
}

// ── TestGetDeadCodeV2 ────────────────────────────────────────────────────────

#[test]
fn v2_returns_dead_symbols() {
    let (_ws, svc, sid) = build_repo();
    let r = dead_code_v2(&svc, &sid, 0.33);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["dead_symbols"].is_array());
}

#[test]
fn v2_dead_symbol_has_required_fields() {
    let (_ws, svc, sid) = build_repo();
    let r = dead_code_v2(&svc, &sid, 0.1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    for sym in r["data"]["dead_symbols"].as_array().unwrap() {
        assert!(sym["id"].is_string(), "missing id");
        assert!(sym["name"].is_string(), "missing name");
        assert!(sym["confidence"].is_number(), "missing confidence");
        assert!(sym["signals"].is_array(), "missing signals");
        let conf = sym["confidence"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&conf));
    }
}

#[test]
fn v2_confidence_respects_threshold() {
    let (_ws, svc, sid) = build_repo();
    let r_high = dead_code_v2(&svc, &sid, 0.9);
    let r_low = dead_code_v2(&svc, &sid, 0.1);
    let high_count = r_high["data"]["dead_symbols"].as_array().unwrap().len();
    let low_count = r_low["data"]["dead_symbols"].as_array().unwrap().len();
    assert!(low_count >= high_count);
}

#[test]
fn v2_total_analysed_present() {
    let (_ws, svc, sid) = build_repo();
    let r = dead_code_v2_default(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["total_analysed"].is_number());
    assert!(r["data"]["total_analysed"].as_u64().is_some());
}

#[test]
fn v2_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "dead_code_v2",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn v2_timing_present() {
    let (_ws, svc, sid) = build_repo();
    let r = dead_code_v2_default(&svc, &sid);
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}

#[test]
fn v2_orphan_file_functions_flagged() {
    let (_ws, svc, sid) = build_repo();
    let r = dead_code_v2(&svc, &sid, 0.33);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead_names: Vec<&str> = r["data"]["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        dead_names.contains(&"orphan_fn"),
        "Expected orphan_fn in {dead_names:?}"
    );
}

#[test]
fn v2_no_invalid_signal_values() {
    let (_ws, svc, sid) = build_repo();
    let valid_signals = ["unreachable_file", "no_callers", "not_barrel_exported"];
    let r = dead_code_v2(&svc, &sid, 0.1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    for sym in r["data"]["dead_symbols"].as_array().unwrap() {
        for sig in sym["signals"].as_array().unwrap() {
            let s = sig.as_str().unwrap();
            assert!(
                valid_signals.contains(&s),
                "Invalid signal '{s}' not in {valid_signals:?}"
            );
        }
    }
}
