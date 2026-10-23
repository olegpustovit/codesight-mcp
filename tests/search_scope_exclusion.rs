use std::fs;

use serde_json::json;
use symslice_engine::config::AppConfig;
use symslice_engine::service::CodeService;
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
fn exclude_drops_symbol_hits() {
    let ws = tempdir().unwrap();
    fs::create_dir_all(ws.path().join("src/keep")).unwrap();
    fs::create_dir_all(ws.path().join("src/skip")).unwrap();
    fs::write(
        ws.path().join("src/keep/a.py"),
        "# TODO keep\ndef shared():\n    return 1\n",
    )
    .unwrap();
    fs::write(
        ws.path().join("src/skip/b.py"),
        "# TODO skip\ndef shared():\n    return 2\n",
    )
    .unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let sid = ingest_code(&svc, ws.path());

    let result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "shared",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "exclude": ["src/skip/"],
            }),
        )
        .unwrap();
    let paths: Vec<String> = result["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.pointer("/ref/path")?.as_str().map(str::to_owned))
        .collect();
    assert!(
        paths.iter().any(|path| path.contains("src/keep")),
        "{paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("src/skip")),
        "{paths:?}"
    );
    assert_eq!(
        result["data"]["total_matches"].as_u64(),
        Some(1),
        "{result:?}"
    );
}

#[test]
fn exclude_drops_file_hits() {
    let ws = tempdir().unwrap();
    fs::create_dir_all(ws.path().join("src/keep")).unwrap();
    fs::create_dir_all(ws.path().join("src/skip")).unwrap();
    fs::write(ws.path().join("src/keep/a.py"), "# TODO keep\n").unwrap();
    fs::write(ws.path().join("src/skip/b.py"), "# TODO skip\n").unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let sid = ingest_code(&svc, ws.path());

    let result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "query_mode": "keyword",
                "unit_kinds": ["file"],
                "exclude": ["src/skip/"],
            }),
        )
        .unwrap();
    let paths: Vec<String> = result["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.pointer("/ref/path")?.as_str().map(str::to_owned))
        .collect();
    assert!(
        paths.iter().any(|path| path.contains("src/keep")),
        "{paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("src/skip")),
        "{paths:?}"
    );
    assert_eq!(
        result["data"]["total_matches"].as_u64(),
        Some(1),
        "{result:?}"
    );
}
