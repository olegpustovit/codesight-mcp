//! E10 — `manage_indexes action=session_stats` surfaces per-tool call counts
//! and token totals sourced from the savings ledger.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_service() -> (CodeService, tempfile::TempDir) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    (svc, ws)
}

#[test]
fn session_stats_tracks_per_tool_calls() {
    let (svc, ws) = build_service();
    // Dispatch two different tools.
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("src").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    let _ = svc.dispatch("catalog_sources", json!({})).unwrap();

    let stats = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    assert!(
        stats["ok"].as_bool().unwrap(),
        "session_stats failed: {stats:?}"
    );

    let per_tool = &stats["data"]["per_tool"];
    assert!(
        per_tool.is_object(),
        "per_tool must be an object: {per_tool}"
    );
    // ingest_sources, catalog_sources, and the session_stats call itself must
    // all be recorded (the session_stats dispatch is itself a tool call).
    for tool in ["ingest_sources", "catalog_sources"] {
        let entry = &per_tool[tool];
        assert!(
            entry.is_object(),
            "missing per_tool entry for {tool}: {per_tool}"
        );
        let calls = entry["calls"].as_u64().unwrap_or(0);
        assert!(calls >= 1, "{tool}: expected ≥1 call, got {calls}");
        // tokens_saved is present (may be zero for trivial payloads).
        assert!(entry["tokens_saved"].is_u64());
    }
    assert!(
        !stats["data"]["started_at"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "started_at should be populated: {stats:?}"
    );
    assert!(
        stats["data"]["session_id"].is_string(),
        "session_id should be populated: {stats:?}"
    );
    assert!(
        stats["data"]["tokens_saved_total"].is_u64(),
        "tokens_saved_total should be present: {stats:?}"
    );
}

#[test]
fn session_stats_does_not_require_source_id() {
    let (svc, _ws) = build_service();
    let r: Value = svc
        .dispatch("manage_indexes", json!({ "action": "session_stats" }))
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "session_stats without source_id should succeed: {r:?}"
    );
}

#[test]
fn session_end_sets_ended_at() {
    let (svc, ws) = build_service();
    let sid = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("src").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap()["data"]["source_id"]
        .as_str()
        .unwrap()
        .to_string();

    let _ = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
            }),
        )
        .unwrap();

    let ended = svc
        .dispatch("manage_indexes", json!({ "action": "session_end" }))
        .unwrap();
    assert!(
        ended["ok"].as_bool().unwrap(),
        "session_end failed: {ended:?}"
    );
    assert!(
        ended["data"]["ended_at"].is_string(),
        "session_end should set ended_at: {ended:?}"
    );
}

#[test]
fn catalog_sources_starts_quality_session_when_source_is_unambiguous() {
    let (svc, ws) = build_service();
    let _sid = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("src").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap()["data"]["source_id"]
        .as_str()
        .unwrap()
        .to_string();

    let catalog = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(
        catalog["ok"].as_bool().unwrap(),
        "catalog_sources failed: {catalog:?}"
    );

    let ended = svc
        .dispatch("manage_indexes", json!({ "action": "session_end" }))
        .unwrap();
    assert!(
        ended["ok"].as_bool().unwrap(),
        "session_end failed: {ended:?}"
    );
    assert!(
        ended["data"]["quality_delta"].is_object(),
        "catalog_sources should have seeded session_start for the only source: {ended:?}"
    );
}
