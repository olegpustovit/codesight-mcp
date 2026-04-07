//! Integration tests: truncated flag.

use std::fs;
use std::path::Path;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn truncated_flag_suite() {
    run_all();
}

fn run_all() {
    run_no_budget_truncated_meta_false();
    run_tiny_budget_truncated_meta_true();
    run_huge_budget_truncated_meta_false();
    run_fuzzy_with_tiny_budget_truncated_meta_true();
    run_token_budget_alias_deserializes();
}

fn make_funcs_repo(parent: &Path) -> std::path::PathBuf {
    let src = parent.join("src");
    fs::create_dir_all(&src).unwrap();
    let mut lines = Vec::new();
    for i in 0..20 {
        lines.push(format!("def budget_test_func_{i}():\n    return {i}\n"));
    }
    fs::write(src.join("funcs.py"), lines.join("")).unwrap();
    src.parent().unwrap().to_path_buf()
}

fn ingest(svc: &CodeService, root: &Path) -> String {
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
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn search_symbols(
    svc: &CodeService,
    sid: &str,
    query: &str,
    limit: usize,
    extra: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut args = serde_json::Map::new();
    args.insert("source_id".to_string(), json!(sid));
    args.insert("query".to_string(), json!(query));
    args.insert("domain".to_string(), json!("code"));
    args.insert("unit_kinds".to_string(), json!(["symbol"]));
    args.insert("limit".to_string(), json!(limit));
    for (k, v) in extra {
        args.insert(k, v);
    }
    svc.dispatch("search_catalog", json!(args)).unwrap()
}

fn run_no_budget_truncated_meta_false() {
    let workspace = tempdir().unwrap();
    let root = make_funcs_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let res = search_symbols(&svc, &sid, "budget_test_func", 50, serde_json::Map::new());
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["_meta"]["truncated"], json!(false));
    let n = res["data"]["items"].as_array().unwrap().len();
    assert!(n >= 10, "expected many symbol hits, got {n}");
}

fn run_tiny_budget_truncated_meta_true() {
    let workspace = tempdir().unwrap();
    let root = make_funcs_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let mut extra = serde_json::Map::new();
    extra.insert("max_tokens".to_string(), json!(1_u32));
    let res = search_symbols(&svc, &sid, "budget_test_func", 50, extra);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["_meta"]["truncated"], json!(true));
}

fn run_huge_budget_truncated_meta_false() {
    let workspace = tempdir().unwrap();
    let root = make_funcs_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let mut extra = serde_json::Map::new();
    extra.insert("max_tokens".to_string(), json!(100_000_u32));
    let res = search_symbols(&svc, &sid, "budget_test_func", 50, extra);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["_meta"]["truncated"], json!(false));
}

fn run_fuzzy_with_tiny_budget_truncated_meta_true() {
    let workspace = tempdir().unwrap();
    let root = make_funcs_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let mut extra = serde_json::Map::new();
    extra.insert("max_tokens".to_string(), json!(1_u32));
    extra.insert("fuzzy".to_string(), json!(true));
    let res = search_symbols(&svc, &sid, "budget_test_func", 50, extra);
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["_meta"]["truncated"], json!(true));
}

fn run_token_budget_alias_deserializes() {
    let workspace = tempdir().unwrap();
    let root = make_funcs_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "budget_test_func",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 50,
                "token_budget": 1,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["_meta"]["truncated"], json!(true));
}
