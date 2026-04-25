//! ROADMAP P1.5 — `context_bundle` depth knob.
//!
//! Depth 0 returns just the user-provided symbol. Depth ≥ 1 pulls in named
//! type references extracted from the signature + body. Depth ≥ 2 adds one
//! representative caller + callee per base symbol. `include_tests: false`
//! drops test-path fixtures from the expansion.

use std::fs;

use codesight_engine::config::AppConfig;
use serde_json::json;
use tempfile::tempdir;

fn build_rust_repo() -> (tempfile::TempDir, codesight_engine::service::CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("repo").join("src");
    fs::create_dir_all(&src).unwrap();
    // Three Rust files wired through a shared type `Widget` so we can
    // verify depth-1 pulls in `Widget` and depth-2 pulls in callers.
    fs::write(
        src.join("widget.rs"),
        "pub struct Widget { pub name: String }\n\
         impl Widget {\n\
         \tpub fn new(name: &str) -> Widget { Widget { name: name.into() } }\n\
         }\n",
    )
    .unwrap();
    fs::write(
        src.join("factory.rs"),
        "use crate::widget::Widget;\n\
         pub fn make_widget(name: &str) -> Widget { Widget::new(name) }\n",
    )
    .unwrap();
    fs::write(
        src.join("consumer.rs"),
        "use crate::factory::make_widget;\n\
         pub fn consume() -> String { make_widget(\"a\").name }\n",
    )
    .unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("repo").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

fn find_symbol_id(svc: &codesight_engine::service::CodeService, sid: &str, q: &str) -> String {
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": q,
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 1,
            }),
        )
        .unwrap();
    r["data"]["items"][0]["ref"]["id"].as_str().unwrap().to_string()
}

#[test]
fn depth_zero_returns_only_the_requested_symbol() {
    let (_ws, svc, sid) = build_rust_repo();
    let sym = find_symbol_id(&svc, &sid, "make_widget");
    let r = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
                "depth": 0,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // depth 0 = single-symbol shape; shape_bundle_response emits the
    // symbol fields at the top level when there's exactly one row.
    assert_eq!(r["data"]["mode"].as_str(), Some("context_bundle"));
    assert!(
        r["data"]["source"].is_string(),
        "depth 0 single-ref shape should expose `source`: {r:?}"
    );
    assert!(
        r["data"].get("symbols").is_none(),
        "depth 0 single-ref shape should not emit multi-row symbols array: {r:?}"
    );
}

#[test]
fn depth_two_expands_with_callers_and_callees() {
    let (_ws, svc, sid) = build_rust_repo();
    let sym = find_symbol_id(&svc, &sid, "make_widget");
    let r = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
                "depth": 2,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let symbols = r["data"]["symbols"].as_array().expect("multi-row shape at depth 2");
    // make_widget + ≥1 expansion (Widget type and/or consume caller).
    assert!(
        symbols.len() >= 2,
        "depth 2 should expand beyond the base symbol: {symbols:?}"
    );
    let names: Vec<&str> = symbols
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        names.contains(&"make_widget"),
        "base symbol must remain: {names:?}"
    );
}

#[test]
fn include_tests_false_drops_test_fixture_rows() {
    let (_ws, svc, sid) = build_rust_repo();
    // Add a test file that references make_widget; without filtering,
    // depth-2 expansion can drag it in.
    let src = _ws.path().join("repo").join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("widget_test.rs"),
        "use repo::factory::make_widget;\n#[test] fn t() { let _ = make_widget(\"a\"); }\n",
    )
    .unwrap();
    // Reindex via incremental=false so the new file is picked up.
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": _ws.path().join("repo").to_string_lossy(),
                "domains": "code",
                "incremental": false,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "reingest failed: {r:?}");
    let sym = find_symbol_id(&svc, &sid, "make_widget");
    let r = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
                "depth": 2,
                "include_tests": false,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let files: Vec<&str> = r["data"]["symbols"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["file"].as_str()).collect())
        .unwrap_or_else(|| {
            vec![r["data"]["file"].as_str().unwrap_or_default()]
        });
    assert!(
        !files.iter().any(|f| f.contains("widget_test")),
        "include_tests=false should drop test-path rows, got {files:?}"
    );
}
