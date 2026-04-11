//! Integration tests: get context bundle.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::indexing::imports::context_bundle_import_lines;
use codesight_mcp::service::CodeService;
use codesight_mcp::test_support;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn get_context_bundle_suite() {
    run_all();
}

fn run_all() {
    case_symbol_source_and_name_kind();
    case_imports_from_file();
    case_invalid_symbol_error();
    case_meta_envelope();
    case_extract_imports_python();
    case_extract_imports_go();
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
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn first_symbol_id(svc: &CodeService, source_id: &str, query: &str) -> String {
    let s = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": query,
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(s["ok"].as_bool().unwrap(), "{s:?}");
    let items = s["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "no symbols for query {query:?}");
    items[0]["ref"]["stable_id"].as_str().unwrap().to_string()
}

fn read_symbol_source(svc: &CodeService, source_id: &str, stable_id: &str) -> String {
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": source_id,
                "refs": [{ "kind": "symbol", "stable_id": stable_id }],
                "include": ["body"],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["items"][0]["source"]
        .as_str()
        .unwrap()
        .to_string()
}

fn case_symbol_source_and_name_kind() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym = first_symbol_id(&svc, &sid, "add");
    let expected_source = read_symbol_source(&svc, &sid, &sym);

    let bundle = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(bundle["ok"].as_bool().unwrap(), "{bundle:?}");
    let d = &bundle["data"];
    assert_eq!(d["source"].as_str().unwrap(), expected_source);
    assert_eq!(d["name"].as_str(), Some("add"));
    assert_eq!(d["kind"].as_str(), Some("function"));
}

fn case_imports_from_file() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("test.py"),
        "import os\nfrom pathlib import Path\n\ndef my_func(x: int) -> str:\n    return str(x)\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym = first_symbol_id(&svc, &sid, "my_func");

    let bundle = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(bundle["ok"].as_bool().unwrap(), "{bundle:?}");
    let imports = bundle["data"]["imports"].as_array().unwrap();
    assert_eq!(imports.len(), 2);
    assert!(
        imports[0].as_str().unwrap().contains("import os"),
        "{imports:?}"
    );
    assert!(
        imports[1]
            .as_str()
            .unwrap()
            .contains("from pathlib import Path"),
        "{imports:?}"
    );
}

fn case_invalid_symbol_error() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);

    let bundle = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": "nope.py::fake#function" }],
            }),
        )
        .unwrap();
    assert!(!bundle["ok"].as_bool().unwrap(), "{bundle:?}");
    let msg = bundle["error"]["message"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(msg.contains("not found"), "{msg}");
}

fn case_meta_envelope() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("repo");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym = first_symbol_id(&svc, &sid, "add");

    let bundle = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(bundle["ok"].as_bool().unwrap(), "{bundle:?}");
    let meta = bundle["_meta"].as_object().unwrap();
    assert!(meta.contains_key("timing_ms"));
    assert!(meta.contains_key("tokens_saved"));
    assert!(meta.contains_key("total_tokens_saved"));
}

fn case_extract_imports_python() {
    let content = "import os\nfrom sys import argv\n\nx = 1\nfrom pathlib import Path\n";
    assert_eq!(
        context_bundle_import_lines(content, "m.py", "python").len(),
        3
    );
}

fn case_extract_imports_go() {
    let content = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {}\n";
    assert_eq!(
        context_bundle_import_lines(content, "main.go", "go").len(),
        4
    );
}
