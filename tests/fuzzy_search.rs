//! Integration tests: fuzzy search.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_retrieval::fuzzy::{edit_distance, trigrams};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn fuzzy_search_suite() {
    case_trigram_unit_tests();
    case_edit_distance_unit_tests();
    case_fuzzy_integration_cases();
}

fn case_trigram_unit_tests() {
    let t = trigrams("hello");
    assert!(t.contains("hel"));
    assert!(t.contains("ell"));
    assert!(t.contains("llo"));
    assert_eq!(t.len(), 3);

    let short = trigrams("ab");
    assert!(short.contains("ab"));

    assert!(trigrams("").is_empty());

    assert_eq!(trigrams("FOO"), trigrams("foo"));
}

fn case_edit_distance_unit_tests() {
    assert_eq!(edit_distance("hello", "hello"), 0);
    assert_eq!(edit_distance("helo", "hello"), 1);
    assert_eq!(edit_distance("hello", "helo"), 1);
    assert_eq!(edit_distance("hello", "hXllo"), 1);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("abc", "xyz"), edit_distance("xyz", "abc"));
    let ed = edit_distance("conn_pool", "connection_pool");
    assert!(ed > 2, "abbreviation distance should exceed 2, got {ed}");
    assert!(edit_distance("authenticate", "autenticate") <= 2);
}

fn setup_fuzzy_demo_repo(parent: &std::path::Path) -> std::path::PathBuf {
    let root = parent.join("fuzzy_demo");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("db.py"),
        "def connection_pool():\n    pass\n\n\
         def get_connection():\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("auth.py"),
        "def authenticate(user, password):\n    pass\n\n\
         def authorize(user, resource):\n    pass\n",
    )
    .unwrap();
    fs::write(src.join("worker.py"), "def fetcher_worker():\n    pass\n").unwrap();
    root
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

fn symbol_names_from_search(data: &serde_json::Value) -> Vec<String> {
    data["items"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|it| {
            it.get("ref")?
                .get("title")?
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .collect()
}

fn case_fuzzy_integration_cases() {
    let workspace = tempdir().unwrap();
    let root = setup_fuzzy_demo_repo(workspace.path());
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "conn_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "fuzzy_threshold": 0.3,
                "limit": 20,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let names = symbol_names_from_search(&res["data"]);
    assert!(
        names.iter().any(|n| n == "connection_pool"),
        "conn_pool fuzzy -> connection_pool, got {names:?}"
    );

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "fetchr_workr",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "fuzzy_threshold": 0.3,
                "limit": 20,
            }),
        )
        .unwrap();
    let items = res["data"]["items"].as_array().unwrap();
    let fuzzy_hits: Vec<_> = items
        .iter()
        .filter(|it| it.get("match_type").and_then(|x| x.as_str()) == Some("fuzzy"))
        .collect();
    assert!(!fuzzy_hits.is_empty(), "expected fuzzy hits");
    let hit = fuzzy_hits
        .iter()
        .find(|it| it["ref"]["title"].as_str() == Some("fetcher_worker"))
        .expect("fetcher_worker fuzzy hit");
    assert!(hit.get("fuzzy_similarity").is_some());
    assert!(hit.get("edit_distance").is_some());
    let sim = hit["fuzzy_similarity"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&sim) && sim > 0.0);

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "authenticate",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "limit": 20,
            }),
        )
        .unwrap();
    let ex: Vec<_> = res["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|it| it.get("match_type").and_then(|x| x.as_str()) == Some("exact"))
        .collect();
    assert!(!ex.is_empty());
    assert_eq!(ex[0]["ref"]["title"].as_str(), Some("authenticate"));

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "authenticate",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": false,
                "limit": 20,
            }),
        )
        .unwrap();
    for it in res["data"]["items"].as_array().unwrap() {
        assert!(it.get("match_type").is_none());
    }

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "conn_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "fuzzy_threshold": 1.0,
                "max_edit_distance": 0,
                "limit": 20,
            }),
        )
        .unwrap();
    let fuzzy_hits: Vec<_> = res["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|it| it.get("match_type").and_then(|x| x.as_str()) == Some("fuzzy"))
        .collect();
    assert!(
        fuzzy_hits
            .iter()
            .all(|it| it["ref"]["title"].as_str() != Some("connection_pool"))
    );

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "autenticate",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "max_edit_distance": 2,
                "limit": 20,
            }),
        )
        .unwrap();
    let names = symbol_names_from_search(&res["data"]);
    assert!(names.iter().any(|n| n == "authenticate"));

    let r1 = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "connection_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 20,
            }),
        )
        .unwrap();
    let r2 = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "connection_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": false,
                "limit": 20,
            }),
        )
        .unwrap();
    let id1: Vec<_> = r1["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["ref"]["id"].as_str().unwrap().to_string())
        .collect();
    let id2: Vec<_> = r2["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["ref"]["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(id1, id2);

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "conn_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": false,
                "fuzzy_threshold": 0.3,
                "limit": 20,
            }),
        )
        .unwrap();
    let names = symbol_names_from_search(&res["data"]);
    assert!(
        names.iter().any(|n| n == "connection_pool"),
        "auto fuzzy when BM25 empty: {names:?}"
    );
}
