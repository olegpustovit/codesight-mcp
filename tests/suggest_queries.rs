//! Integration tests: suggest queries.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_mcp::test_support;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn suggest_queries_suite() {
    run_all();
}

fn run_all() {
    case_repo_not_indexed_error();
    case_empty_index_error();
    case_small_counts_and_distributions();
    case_small_example_queries();
    case_medium_files_repo_meta();
    case_medium_kind_distribution();
    case_most_imported_structure();
}

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    ing["data"]["source_id"].as_str().unwrap().to_string()
}

fn dispatch_suggestions(svc: &CodeService, source_id: &str) -> serde_json::Value {
    svc.dispatch(
        "assemble_context",
        json!({
            "source_id": source_id,
            "mode": "query_suggestions",
        }),
    )
    .unwrap()
}

fn case_repo_not_indexed_error() {
    let workspace = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let res = dispatch_suggestions(&svc, "no_such_repo_xyz");
    assert!(!res["ok"].as_bool().unwrap(), "{res:?}");
    assert!(res.get("error").is_some());
}

fn case_empty_index_error() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("README.md"), "# Hello\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(!res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["error"]["code"].as_str(), Some("EMPTY_INDEX"));
}

fn case_small_counts_and_distributions() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let d = &res["data"];
    assert_eq!(d["symbol_count"].as_u64(), Some(3));
    assert_eq!(d["file_count"].as_u64(), Some(1));
    let kd = d["kind_distribution"].as_object().unwrap();
    assert_eq!(kd.get("function").and_then(|x| x.as_u64()), Some(2));
    let ld = d["language_distribution"].as_object().unwrap();
    assert!(ld.contains_key("python"));
}

fn case_small_example_queries() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let eq = res["data"]["example_queries"].as_array().unwrap();
    assert!(!eq.is_empty());
    for item in eq {
        assert!(item.get("query").is_some());
        assert!(item.get("tool").is_some());
        assert!(item.get("description").is_some());
    }
}

fn case_medium_files_repo_meta() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_medium_repo(&src).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["data"]["file_count"].as_u64(), Some(3));
    assert!(res["data"].get("repo").is_some());
    let meta = res["_meta"].as_object().unwrap();
    assert!(meta["timing_ms"].is_number());
}

fn case_medium_kind_distribution() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_medium_repo(&src).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let kd = res["data"]["kind_distribution"].as_object().unwrap();
    assert!(kd.contains_key("class"));
    assert!(kd.contains_key("function"));
}

fn case_most_imported_structure() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_medium_repo(&src).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = dispatch_suggestions(&svc, &sid);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let files = res["data"]["most_imported_files"].as_array().unwrap();
    for entry in files {
        assert!(entry.get("file").is_some());
        let count = entry.get("imported_by").unwrap().as_u64().unwrap();
        assert!(count >= 1);
    }
}
