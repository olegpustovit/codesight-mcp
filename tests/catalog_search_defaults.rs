//! Lever 6a/b — `search_catalog` auto-routes `domain` from `query_mode`
//! and surfaces the effective selection in `_meta`.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_service_with_code() -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
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

fn dispatch_search(svc: &CodeService, args: Value) -> Value {
    let r = svc.dispatch("search_catalog", args).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "search failed: {r:?}");
    r
}

#[test]
fn default_query_mode_keyword_routes_to_code() {
    let (svc, _ws, sid) = build_service_with_code();
    // Slim _meta suppresses defaults: domain=code + query_mode=keyword +
    // domain_explicit=false all match the default, so none appear unless the
    // caller explicitly requests verbose mode.
    let r = dispatch_search(
        &svc,
        json!({ "source_id": sid, "query": "hello", "limit": 5, "_meta_verbose": true }),
    );
    assert_eq!(r["_meta"]["domain_effective"], "code");
    assert_eq!(r["_meta"]["query_mode_effective"], "keyword");
    assert_eq!(r["_meta"]["domain_explicit"], false);
}

#[test]
fn explicit_keyword_routes_to_code() {
    let (svc, _ws, sid) = build_service_with_code();
    let r = dispatch_search(
        &svc,
        json!({
            "source_id": sid,
            "query": "hello",
            "query_mode": "keyword",
            "limit": 5,
            "_meta_verbose": true,
        }),
    );
    assert_eq!(r["_meta"]["domain_effective"], "code");
    assert_eq!(r["_meta"]["query_mode_effective"], "keyword");
}

#[test]
fn regex_routes_to_code() {
    let (svc, _ws, sid) = build_service_with_code();
    // `regex` is non-default and survives the slim filter, but domain=code is
    // default — verify with verbose to assert both values.
    let r = dispatch_search(
        &svc,
        json!({
            "source_id": sid,
            "query": "hello",
            "query_mode": "regex",
            "limit": 5,
            "_meta_verbose": true,
        }),
    );
    assert_eq!(r["_meta"]["domain_effective"], "code");
    assert_eq!(r["_meta"]["query_mode_effective"], "regex");
}

#[test]
fn natural_language_routes_to_hybrid() {
    let (svc, _ws, sid) = build_service_with_code();
    // hybrid + natural_language + domain_explicit=false — only the non-default
    // ones survive the slim filter. domain_explicit=false matches the default
    // so verify it via verbose.
    let r = dispatch_search(
        &svc,
        json!({
            "source_id": sid,
            "query": "what does hello return",
            "query_mode": "natural_language",
            "limit": 5,
            "_meta_verbose": true,
        }),
    );
    assert_eq!(r["_meta"]["domain_effective"], "hybrid");
    assert_eq!(r["_meta"]["query_mode_effective"], "natural_language");
    assert_eq!(r["_meta"]["domain_explicit"], false);
}

#[test]
fn explicit_domain_wins_over_autoroute() {
    let (svc, _ws, sid) = build_service_with_code();
    // domain_explicit=true is non-default so it stays in the slim envelope,
    // but domain_effective=code matches the default and is stripped — use
    // verbose to assert both.
    let r = dispatch_search(
        &svc,
        json!({
            "source_id": sid,
            "query": "hello",
            "query_mode": "natural_language",
            "domain": "code",
            "limit": 5,
            "_meta_verbose": true,
        }),
    );
    assert_eq!(r["_meta"]["domain_effective"], "code");
    assert_eq!(r["_meta"]["domain_explicit"], true);
}
