//! Envelope-shape regression tests for the slim default `_meta` contract.

use std::collections::BTreeSet;
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
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    (svc, ws)
}

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
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn meta_keys(response: &Value) -> BTreeSet<String> {
    response["_meta"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn catalog_sources_default_meta_is_minimal_and_session_carries_constants() {
    let (svc, _ws) = build_service();
    let r = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "catalog_sources failed: {r:?}");

    let keys = meta_keys(&r);
    assert!(keys.contains("timing_ms"));
    // catalog_sources has no file-read baseline → tokens_saved is omitted
    // so agents don't read a misleading zero.
    assert!(
        !keys.contains("tokens_saved"),
        "catalog_sources should omit tokens_saved: {keys:?}"
    );
    assert!(!keys.contains("build_sha"), "default meta should be slim: {keys:?}");

    let session = &r["session"];
    assert!(session["id"].is_string(), "session id missing: {r:?}");
    assert!(session["started_at"].is_string(), "session start missing: {r:?}");
    assert!(session["build_sha"].is_string(), "session build_sha missing: {r:?}");
    assert!(
        session["retrieval_version"].is_string(),
        "session retrieval_version missing: {r:?}"
    );
}

#[test]
fn search_catalog_verbose_meta_restores_full_build_fields() {
    let (svc, ws) = build_service();
    let sid = ingest(&svc, &ws.path().join("src"));

    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "search_catalog failed: {r:?}");

    let keys = meta_keys(&r);
    assert!(keys.contains("build_sha"));
    assert!(keys.contains("build_time"));
    assert!(keys.contains("retrieval_version"));
    assert!(keys.contains("total_tokens_saved"));
}

#[test]
fn error_envelope_keeps_error_category_but_not_full_build_meta_by_default() {
    let (svc, _ws) = build_service();
    let r = svc.dispatch("search_catalog", json!({ "source_id": "missing" })).unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected failure: {r:?}");

    let keys = meta_keys(&r);
    assert!(keys.contains("error_category"));
    assert!(!keys.contains("build_sha"), "default error meta should be slim: {keys:?}");
    assert_eq!(r["error"]["category"], r["_meta"]["error_category"]);
}
