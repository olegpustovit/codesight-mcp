//! Integration tests: related symbols.

use std::collections::HashSet;
use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::fuzzy::tokenize_name;
use codesight_mcp::service::CodeService;
use codesight_mcp::test_support;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn related_symbols_suite() {
    case_tokenize_name_unit();
    case_repo_and_symbol_errors();
    case_small_index_related();
    case_name_token_repo();
    case_medium_cap();
    case_meta_and_fields();
}

fn case_tokenize_name_unit() {
    let expected: HashSet<_> = ["get", "user"].into_iter().map(String::from).collect();
    assert_eq!(tokenize_name("get_user"), expected);
    let gubi = tokenize_name("getUserById");
    assert!(
        gubi.contains("get") && gubi.contains("user") && gubi.contains("by") && gubi.contains("id")
    );
    let expected: HashSet<_> = ["process"].into_iter().map(String::from).collect();
    assert_eq!(tokenize_name("process"), expected);
    let abc = tokenize_name("a_b_c");
    assert!(!abc.contains("a") && !abc.contains("b") && !abc.contains("c"));
    for t in tokenize_name("ParseFile") {
        assert_eq!(t, t.to_ascii_lowercase());
    }
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

fn find_symbol_id(svc: &CodeService, source_id: &str, name: &str) -> String {
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": name,
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 20,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    for it in res["data"]["items"].as_array().unwrap() {
        if it["ref"]["title"].as_str() == Some(name) {
            return it["ref"]["id"].as_str().unwrap().to_string();
        }
    }
    panic!("symbol {name} not found");
}

fn case_repo_and_symbol_errors() {
    let workspace = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "no_such_repo___",
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": "x" },
            }),
        )
        .unwrap();
    assert!(!res["ok"].as_bool().unwrap());

    let root = workspace.path().join("r");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let sid = ingest(&svc, &root);

    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": "nonexistent.py::ghost#function" },
            }),
        )
        .unwrap();
    assert!(!res["ok"].as_bool().unwrap());
    let msg = res["error"]["message"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(msg.contains("not found"));
}

fn case_small_index_related() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("r");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym_id = find_symbol_id(&svc, &sid, "add");

    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": sym_id },
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let related = res["data"]["related"].as_array().unwrap();
    assert!(!related.is_empty());
    let names: Vec<_> = related.iter().filter_map(|x| x["name"].as_str()).collect();
    assert!(names.contains(&"subtract"));

    for e in related {
        assert!(e["relatedness_score"].as_f64().unwrap_or(0.0) > 0.0);
    }
}

fn case_name_token_repo() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("r");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("users.py"),
        "def get_user(uid):\n    return uid\n\n\
         def get_user_profile(uid):\n    return {}\n\n\
         def delete_order(oid):\n    return True\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym_id = find_symbol_id(&svc, &sid, "get_user");
    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": sym_id },
                "limit": 25,
            }),
        )
        .unwrap();
    let names: Vec<_> = res["data"]["related"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(names.contains(&"get_user_profile"));
}

fn case_medium_cap() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("r");
    let src = root.join("src");
    test_support::write_medium_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let any_id = find_symbol_id(&svc, &sid, "User");
    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": any_id },
                "limit": 2,
            }),
        )
        .unwrap();
    assert!(res["data"]["related"].as_array().unwrap().len() <= 2);
}

fn case_meta_and_fields() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("r");
    let src = root.join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let sym_id = find_symbol_id(&svc, &sid, "add");
    let res = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "related",
                "ref": { "kind": "symbol", "stable_id": sym_id },
            }),
        )
        .unwrap();
    assert!(res.get("_meta").is_some());
    assert!(res["_meta"].get("timing_ms").is_some());

    let sym = &res["data"]["symbol"];
    assert_eq!(sym["name"].as_str(), Some("add"));
    assert_eq!(sym["id"].as_str(), Some(sym_id.as_str()));

    for e in res["data"]["related"].as_array().unwrap() {
        assert!(e.get("id").is_some());
        assert!(e.get("name").is_some());
        assert!(e.get("kind").is_some());
        assert!(e.get("file").is_some());
        assert!(e.get("relatedness_score").is_some());
    }
}
