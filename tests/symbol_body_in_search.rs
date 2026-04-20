use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn ingest(svc: &CodeService, root: &std::path::Path, domains: &str) -> String {
    let response = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": domains,
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");
    response["data"]["source_id"].as_str().unwrap().to_string()
}

#[test]
fn symbol_body_included_with_context() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("m.py"),
        "# pre\ndef target():\n    return 'v'\n# post\n",
    )
    .unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path(), "code");

    let legacy = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "target",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(
        legacy["data"]["items"][0].get("body").is_none(),
        "{legacy:?}"
    );

    let enriched = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "target",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
                "include_body": true,
                "context_lines": 1,
            }),
        )
        .unwrap();
    let body = enriched["data"]["items"][0]["body"].as_str().unwrap();
    assert!(body.contains("def target"), "{body}");
    assert!(body.contains("# pre"), "{body}");
}

#[test]
fn section_body_included_with_context() {
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("README.md"),
        "intro\n# Example\nbody line\ntrailer\n",
    )
    .unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path(), "docs");

    let enriched = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "domain": "docs",
                "query": "Example",
                "query_mode": "keyword",
                "unit_kinds": ["section"],
                "limit": 5,
                "include_body": true,
                "context_lines": 1,
            }),
        )
        .unwrap();
    let body = enriched["data"]["items"][0]["body"].as_str().unwrap();
    assert!(body.contains("# Example"), "{body}");
    assert!(body.contains("intro"), "{body}");
    assert!(body.contains("body line"), "{body}");
}
