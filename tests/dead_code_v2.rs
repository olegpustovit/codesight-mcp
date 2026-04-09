//! Slice tests for dead-code-v2, plus a small MCP-envelope contract check.

use codesight_mcp::analysis::dead_code::dead_code_v2_payload;
use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::IndexStore;
use codesight_mcp::test_support::{index_code_fixture, open_code_store, write_files};
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_repo() -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            (
                "main.py",
                "from utils import used_fn\n\nif __name__ == '__main__':\n    used_fn()\n",
            ),
            (
                "utils.py",
                "def used_fn():\n    return 1\n\ndef dead_fn():\n    return 2\n",
            ),
            ("orphan.py", "def orphan_fn():\n    pass\n"),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

fn dead_code_v2(store: &IndexStore, min_confidence: f64) -> Value {
    dead_code_v2_payload(store, min_confidence).unwrap()
}

#[test]
fn v2_returns_dead_symbols() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = dead_code_v2(&store, 0.33);
    assert!(r["dead_symbols"].is_array());
}

#[test]
fn v2_dead_symbol_has_required_fields() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = dead_code_v2(&store, 0.1);
    for sym in r["dead_symbols"].as_array().unwrap() {
        assert!(sym["id"].is_string(), "missing id");
        assert!(sym["name"].is_string(), "missing name");
        assert!(sym["confidence"].is_number(), "missing confidence");
        assert!(sym["signals"].is_array(), "missing signals");
        assert!(sym["evidence_notes"].is_array(), "missing evidence_notes");
        let conf = sym["confidence"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&conf));
    }
}

#[test]
fn v2_confidence_respects_threshold() {
    let (_ws, _config, _sid, store) = build_repo();
    let r_high = dead_code_v2(&store, 0.9);
    let r_low = dead_code_v2(&store, 0.1);
    assert!(
        r_low["dead_symbols"].as_array().unwrap().len()
            >= r_high["dead_symbols"].as_array().unwrap().len()
    );
}

#[test]
fn v2_total_analysed_present() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = dead_code_v2(&store, 0.5);
    assert!(r["total_analysed"].is_number());
    assert!(r["total_analysed"].as_u64().is_some());
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
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            ("main.py", "if __name__ == '__main__': pass"),
            ("orphan.py", "def orphan_fn():\n    pass\n"),
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
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code_v2",
            }),
        )
        .unwrap();
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}

#[test]
fn v2_orphan_file_functions_flagged() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = dead_code_v2(&store, 0.33);
    let dead_names: Vec<&str> = r["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(dead_names.contains(&"orphan_fn"), "{dead_names:?}");
}

#[test]
fn v2_no_invalid_signal_values() {
    let (_ws, _config, _sid, store) = build_repo();
    let valid_signals = ["unreachable_file", "no_callers", "not_barrel_exported"];
    let r = dead_code_v2(&store, 0.1);
    for sym in r["dead_symbols"].as_array().unwrap() {
        for sig in sym["signals"].as_array().unwrap() {
            let signal = sig.as_str().unwrap();
            assert!(
                valid_signals.contains(&signal),
                "invalid signal '{signal}' not in {valid_signals:?}"
            );
        }
    }
}
