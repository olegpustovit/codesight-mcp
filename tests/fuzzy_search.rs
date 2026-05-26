//! Integration tests: fuzzy search (nucleo-matcher backed).

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_retrieval::fuzzy::{fuzzy_rank, fuzzy_score};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn fuzzy_search_suite() {
    case_fuzzy_unit_tests();
    case_fuzzy_integration_cases();
}

fn case_fuzzy_unit_tests() {
    assert_eq!(fuzzy_score("hello", "hello"), Some(1.0));
    assert!(fuzzy_score("helo", "hello").is_some());
    assert!(fuzzy_score("xyz", "abc").is_none());

    let ranked = fuzzy_rank(
        "conn_pool",
        ["connection_pool", "get_connection", "unrelated"]
            .iter()
            .copied(),
    );
    assert!(!ranked.is_empty());
    // `connection_pool` is the closest subsequence match.
    let first_name = ["connection_pool", "get_connection", "unrelated"][ranked[0].0];
    assert_eq!(first_name, "connection_pool");
}

#[test]
fn typo_finds_nearest_symbol() {
    // `SavingLedger` is one char off from `SavingsLedger`. Nucleo must
    // put SavingsLedger at the top, not a loosely-related distractor.
    let names = [
        "savings_file",
        "savings_entry",
        "SavingsLedger",
        "unrelated_helper",
    ];
    let ranked = fuzzy_rank("SavingLedger", names.iter().copied());
    let first = ranked.first().expect("at least one match");
    assert_eq!(
        names[first.0], "SavingsLedger",
        "expected SavingsLedger first, got {:?}",
        ranked
    );

    // Regression: an earlier revision of `compute_fuzzy_lane` clamped the
    // light-scan at 1 250 rows, so symbols in late-sorting files silently
    // dropped out of the fuzzy lane end-to-end — the response missed a
    // `data.fuzzy` key entirely. Dispatch through `search_catalog` and
    // assert the lane array is present and carries the nucleo-ranked hit.
    let workspace = tempdir().unwrap();
    let root = setup_fuzzy_demo_repo(workspace.path());
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "fetchr_workr",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "limit": 20,
            }),
        )
        .unwrap();
    let fuzzy = res["data"]["fuzzy"]
        .as_array()
        .unwrap_or_else(|| panic!("`data.fuzzy` array missing when fuzzy=true; got {res:?}"));
    assert!(
        fuzzy
            .iter()
            .any(|it| it["ref"]["title"].as_str() == Some("fetcher_worker")),
        "`data.fuzzy` must carry the nucleo hit for the typo, got {fuzzy:?}"
    );

    // Tie-breaker assertion: querying `SavingLedger` must surface the
    // `SavingsLedger` struct within the top 3 fuzzy hits. Without the
    // length + kind-priority tie-breakers, six longer `savings_ledger_*`
    // methods tied at 1.0 crowded the struct out to rank 7+.
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "SavingLedger",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
                "limit": 20,
            }),
        )
        .unwrap();
    let fuzzy = res["data"]["fuzzy"].as_array().cloned().unwrap_or_default();
    let top3: Vec<&str> = fuzzy
        .iter()
        .take(3)
        .filter_map(|it| it["ref"]["title"].as_str())
        .collect();
    assert!(
        top3.contains(&"SavingsLedger"),
        "expected `SavingsLedger` in top-3 fuzzy hits for `SavingLedger`, got top3={top3:?} full={fuzzy:?}"
    );
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
    // Rust file exercising the fuzzy tie-breakers: a short struct name plus
    // several longer method names that share the same subsequence of the
    // query `SavingLedger`. With length + kind-priority tie-breakers the
    // struct should rank ahead of the method distractors.
    fs::write(
        src.join("savings.rs"),
        "pub struct SavingsLedger;\n\
         impl SavingsLedger {\n\
             pub fn savings_ledger_end_session(&self) {}\n\
             pub fn savings_ledger_current_session(&self) {}\n\
             pub fn savings_ledger_record_stale_warning(&self) {}\n\
             pub fn savings_ledger_set_quality_session_end(&self) {}\n\
             pub fn savings_ledger_set_quality_session_start(&self) {}\n\
             pub fn savings_ledger_rounds_accumulated_costs(&self) {}\n\
         }\n",
    )
    .unwrap();
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
    let from_lane = |key: &str| -> Vec<String> {
        data.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| {
                        it.get("ref")?
                            .get("title")?
                            .as_str()
                            .map(std::string::ToString::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = from_lane("items");
    out.extend(from_lane("fuzzy"));
    out
}

fn case_fuzzy_integration_cases() {
    let workspace = tempdir().unwrap();
    let root = setup_fuzzy_demo_repo(workspace.path());
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
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
                "limit": 20,
            }),
        )
        .unwrap();
    let fuzzy_hits = res["data"]["fuzzy"].as_array().cloned().unwrap_or_default();
    assert!(!fuzzy_hits.is_empty(), "expected fuzzy hits");
    let hit = fuzzy_hits
        .iter()
        .find(|it| it["ref"]["title"].as_str() == Some("fetcher_worker"))
        .expect("fetcher_worker fuzzy hit");
    assert!(hit.get("subsequence_score").is_some());
    let sim = hit["subsequence_score"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&sim) && sim > 0.0);
    // Companion field: edit_similarity (Jaro-Winkler) is also surfaced.
    assert!(
        hit.get("edit_similarity").is_some(),
        "edit_similarity should accompany subsequence_score on fuzzy hits"
    );

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
                "query": "autenticate",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": true,
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

    // Auto-fuzzy: BM25 returns nothing → the fuzzy lane promotes rows
    // into `items` even without `fuzzy=true`.
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "conn_pool",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "fuzzy": false,
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
