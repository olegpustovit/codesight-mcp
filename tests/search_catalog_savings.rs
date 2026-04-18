use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn ingest_code(svc: &CodeService, root: &std::path::Path) -> String {
    let response = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");
    response["data"]["source_id"].as_str().unwrap().to_string()
}

#[test]
fn search_catalog_savings_use_candidate_files_not_repo_bytes() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    let matching = "def hello():\n    return 1\n";
    fs::write(src.join("main.py"), matching).unwrap();
    fs::write(src.join("huge_blob.py"), "x".repeat(400_000)).unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_code(&svc, ws.path());

    let response = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");

    let naive_tokens = response["_meta"]["naive_tokens"].as_u64().unwrap();
    assert_eq!(
        naive_tokens,
        (matching.len() as u64) / 4,
        "search_catalog should estimate against the matched candidate file, not the whole repo: {response:?}"
    );
}

#[test]
fn search_catalog_count_mode_uses_the_same_candidate_file_baseline() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    let matching = "def hello():\n    return 1\n";
    fs::write(src.join("main.py"), matching).unwrap();
    fs::write(src.join("huge_blob.py"), "y".repeat(400_000)).unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_code(&svc, ws.path());

    let response = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "output_mode": "count",
                "limit": 5,
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");

    let naive_tokens = response["_meta"]["naive_tokens"].as_u64().unwrap();
    assert_eq!(
        naive_tokens,
        (matching.len() as u64) / 4,
        "count mode should use the same candidate-file savings baseline: {response:?}"
    );
}
