use std::collections::BTreeSet;
use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn build_repo() -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    for idx in 0..8 {
        fs::write(
            src.join(format!("file_{idx}.py")),
            format!("def hello_{idx}():\n    return 'hello world {idx}'\n"),
        )
        .unwrap();
    }
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (svc, ws, sid)
}

fn hit_keys(response: &serde_json::Value) -> BTreeSet<(String, u64)> {
    response["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["ref"]["path"].as_str().unwrap().to_string(),
                item["line"].as_u64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn regex_offset_paginates_without_repeating_hits() {
    let (svc, _ws, sid) = build_repo();
    let first = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello world",
                "query_mode": "regex",
                "limit": 3,
                "offset": 0,
            }),
        )
        .unwrap();
    let second = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello world",
                "query_mode": "regex",
                "limit": 3,
                "offset": 3,
            }),
        )
        .unwrap();

    let first_hits = hit_keys(&first);
    let second_hits = hit_keys(&second);
    assert_eq!(first_hits.len(), 3);
    assert_eq!(second_hits.len(), 3);
    assert!(
        first_hits.is_disjoint(&second_hits),
        "offset pages should not repeat hits: first={first_hits:?} second={second_hits:?}"
    );
}
