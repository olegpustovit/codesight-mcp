//! Integration tests: pagerank.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use codesight_analysis::graph::{compute_in_out_degrees, compute_pagerank};
use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn index_code(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

/// Build a 4-file repo where core.py is imported by both utils.py and app.py.
fn build_centrality_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("core.py"), "def central_func():\n    return 42\n").unwrap();
    fs::write(
        src.join("utils.py"),
        "from core import central_func\n\ndef util_helper():\n    return central_func()\n",
    )
    .unwrap();
    fs::write(
        src.join("app.py"),
        "from core import central_func\n\ndef main():\n    return central_func()\n",
    )
    .unwrap();
    fs::write(
        src.join("standalone.py"),
        "def isolated():\n    return 'no imports'\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

fn inspect_centrality(
    svc: &CodeService,
    sid: &str,
    algorithm: Option<&str>,
    top_n: Option<usize>,
) -> Value {
    let mut params = json!({
        "source_id": sid,
        "graph": "centrality",
    });
    if let Some(a) = algorithm {
        params["algorithm"] = json!(a);
    }
    if let Some(n) = top_n {
        params["top_n"] = json!(n);
    }
    svc.dispatch("inspect_graph", params).unwrap()
}

// ===========================================================================
// Unit tests: compute_pagerank  (TestComputePagerank, 8 tests)
// ===========================================================================

#[test]
fn pagerank_scores_sum_to_one() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec![]),
        ("c.py".into(), vec!["b.py".into()]),
        ("d.py".into(), vec!["b.py".into()]),
    ]);
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    let sum: f64 = pr.scores.values().sum();
    assert!(
        (sum - 1.0).abs() < 0.01,
        "scores should sum to ~1.0, got {sum}"
    );
}

#[test]
fn pagerank_most_imported_ranks_highest() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec![]),
        ("c.py".into(), vec!["b.py".into()]),
    ]);
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    let score_b = pr.scores["b.py"];
    let score_a = pr.scores["a.py"];
    let score_c = pr.scores["c.py"];
    assert!(
        score_b > score_a,
        "b.py ({score_b}) should rank higher than a.py ({score_a})"
    );
    assert!(
        score_b > score_c,
        "b.py ({score_b}) should rank higher than c.py ({score_c})"
    );
}

#[test]
fn pagerank_empty_import_graph() {
    let adj: HashMap<String, Vec<String>> =
        HashMap::from([("a.py".into(), vec![]), ("b.py".into(), vec![])]);
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    let score_a = pr.scores["a.py"];
    let score_b = pr.scores["b.py"];
    assert!(
        (score_a - 0.5).abs() < 0.01,
        "uniform distribution expected, a={score_a}"
    );
    assert!(
        (score_b - 0.5).abs() < 0.01,
        "uniform distribution expected, b={score_b}"
    );
}

#[test]
fn pagerank_empty_file_list() {
    let adj: HashMap<String, Vec<String>> = HashMap::new();
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    assert!(pr.scores.is_empty());
    assert_eq!(pr.iterations, 0);
}

#[test]
fn pagerank_convergence_reported() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec!["c.py".into()]),
        ("c.py".into(), vec!["a.py".into()]),
    ]);
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    assert!(pr.iterations >= 1, "should iterate at least once");
    assert!(pr.iterations <= 100, "should converge within max_iter");
}

#[test]
fn pagerank_in_out_degrees() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec![]),
        ("c.py".into(), vec!["b.py".into()]),
        ("d.py".into(), vec!["b.py".into()]),
    ]);
    let degrees = compute_in_out_degrees(&adj);
    assert_eq!(degrees["b.py"].in_degree, 3, "b.py should have in_degree=3");
    assert_eq!(
        degrees["b.py"].out_degree, 0,
        "b.py should have out_degree=0"
    );
    assert_eq!(degrees["a.py"].in_degree, 0, "a.py should have in_degree=0");
    assert_eq!(
        degrees["a.py"].out_degree, 1,
        "a.py should have out_degree=1"
    );
}

#[test]
fn pagerank_self_import_ignored() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["a.py".into(), "b.py".into()]),
        ("b.py".into(), vec![]),
    ]);
    let degrees = compute_in_out_degrees(&adj);
    assert_eq!(
        degrees["a.py"].in_degree, 0,
        "self-import should not count: in_deg[a.py] should be 0"
    );
    assert_eq!(
        degrees["a.py"].out_degree, 1,
        "self-import excluded, only b.py counts: out_deg[a.py] should be 1"
    );
}

#[test]
fn pagerank_circular_imports_no_infinite_loop() {
    let adj: HashMap<String, Vec<String>> = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec!["c.py".into()]),
        ("c.py".into(), vec!["a.py".into()]),
    ]);
    let pr = compute_pagerank(&adj, 0.85, 100, 1e-6);
    assert!(pr.iterations <= 100, "should converge within max_iter");
    let sum: f64 = pr.scores.values().sum();
    assert!(sum > 0.0, "scores should be positive");
}

// ===========================================================================
// Integration tests: get_symbol_importance  (TestGetSymbolImportance, 8 tests)
// ===========================================================================

#[test]
fn importance_most_imported_symbol_ranks_first() {
    let (_ws, svc, sid) = build_centrality_repo();
    let r = inspect_centrality(&svc, &sid, Some("pagerank"), Some(10));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let ranked = r["data"]["ranked_files"].as_array().unwrap();
    assert!(!ranked.is_empty(), "should have ranked files");
    // core.py is imported by 2 files, so it should rank first
    let top_path = ranked[0]["path"].as_str().unwrap();
    assert!(
        top_path.contains("core.py"),
        "top ranked should be core.py, got '{top_path}'"
    );
}

#[test]
fn importance_response_shape() {
    let (_ws, svc, sid) = build_centrality_repo();
    let r = inspect_centrality(&svc, &sid, Some("pagerank"), Some(10));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(data["ranked_files"].is_array());
    assert_eq!(data["algorithm"].as_str().unwrap(), "pagerank");
    assert!(data["iterations_to_converge"].is_number());

    let first = &data["ranked_files"][0];
    assert!(first["path"].is_string(), "should have path");
    assert!(first["rank"].is_number(), "should have rank");
    assert!(first["score"].is_number(), "should have score");
    assert!(first["in_degree"].is_number(), "should have in_degree");
    assert!(first["out_degree"].is_number(), "should have out_degree");
    assert!(first["kind"].is_string(), "should have kind");
}

#[test]
fn importance_degree_algorithm() {
    let (_ws, svc, sid) = build_centrality_repo();
    let r = inspect_centrality(&svc, &sid, Some("degree"), Some(10));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert_eq!(data["iterations_to_converge"].as_u64().unwrap(), 0);
    let ranked = data["ranked_files"].as_array().unwrap();
    assert!(!ranked.is_empty());
}

#[test]
fn importance_degree_same_top_as_pagerank() {
    let (_ws, svc, sid) = build_centrality_repo();
    let pr_result = inspect_centrality(&svc, &sid, Some("pagerank"), Some(1));
    let deg_result = inspect_centrality(&svc, &sid, Some("degree"), Some(1));
    assert!(pr_result["ok"].as_bool().unwrap());
    assert!(deg_result["ok"].as_bool().unwrap());

    let pr_top = pr_result["data"]["ranked_files"][0]["path"]
        .as_str()
        .unwrap();
    let deg_top = deg_result["data"]["ranked_files"][0]["path"]
        .as_str()
        .unwrap();
    assert_eq!(
        pr_top, deg_top,
        "on simple graph, pagerank and degree should agree on top-1"
    );
}

#[test]
fn importance_invalid_algorithm() {
    let (_ws, svc, sid) = build_centrality_repo();
    let r = inspect_centrality(&svc, &sid, Some("bogus_algo"), None);
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "invalid algorithm should return error: {r:?}"
    );
}

#[test]
fn importance_top_n_respected() {
    let (_ws, svc, sid) = build_centrality_repo();
    let r = inspect_centrality(&svc, &sid, Some("pagerank"), Some(1));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let ranked = r["data"]["ranked_files"].as_array().unwrap();
    assert!(ranked.len() <= 1, "top_n=1 should return at most 1");
}

#[test]
fn importance_missing_repo() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "nonexistent/repo",
                "graph": "centrality",
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "missing repo should return error: {r:?}"
    );
}

// ===========================================================================
// Integration tests: search_symbols sort_by  (TestSearchSymbolsSortBy, 5 tests)
// ===========================================================================

/// Build a repo matching the Python test fixture for sort_by tests:
/// core.py (class Engine, def run), utils.py (imports core, def helper),
/// app.py (imports core+utils, def main).
fn build_sort_by_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("core.py"),
        "class Engine:\n    def run(self):\n        pass\n",
    )
    .unwrap();
    fs::write(
        src.join("utils.py"),
        "from core import Engine\n\ndef helper():\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("app.py"),
        "from core import Engine\nfrom utils import helper\n\ndef main():\n    pass\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

fn search_with_sort(svc: &CodeService, sid: &str, query: &str, sort_by: Option<&str>) -> Value {
    let mut params = json!({
        "source_id": sid,
        "query": query,
    });
    if let Some(sb) = sort_by {
        params["sort_by"] = json!(sb);
    }
    svc.dispatch("search_catalog", params).unwrap()
}

#[test]
fn search_sort_relevance_is_default() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = search_with_sort(&svc, &sid, "Engine", Some("relevance"));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have results");
}

#[test]
fn search_sort_centrality() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = search_with_sort(&svc, &sid, "Engine", Some("centrality"));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have results");
}

#[test]
fn search_sort_combined() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = search_with_sort(&svc, &sid, "helper", Some("combined"));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have results");
}

#[test]
fn search_sort_invalid_sort_by() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = search_with_sort(&svc, &sid, "Engine", Some("magic"));
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "invalid sort_by should return error: {r:?}"
    );
}

#[test]
fn search_sort_stable_order() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r1 = search_with_sort(&svc, &sid, "Engine", None);
    let r2 = search_with_sort(&svc, &sid, "Engine", Some("relevance"));
    assert!(r1["ok"].as_bool().unwrap(), "{r1:?}");
    assert!(r2["ok"].as_bool().unwrap(), "{r2:?}");
    let ids1: Vec<&str> = r1["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i.pointer("/ref/id").and_then(|v| v.as_str()))
        .collect();
    let ids2: Vec<&str> = r2["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i.pointer("/ref/id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(ids1, ids2, "relevance sort should match default sort");
}

// ===========================================================================
// Integration tests: get_repo_outline most_central_symbols
//   (TestRepoOutlineMostCentralSymbols, 3 tests)
// ===========================================================================

fn get_repo_outline(svc: &CodeService, sid: &str) -> Value {
    svc.dispatch(
        "inspect_structure",
        json!({
            "source_id": sid,
            "view": "repo_outline",
        }),
    )
    .unwrap()
}

#[test]
fn outline_most_central_symbols_present() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = get_repo_outline(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"]["most_central_symbols"].is_array(),
        "repo_outline should include most_central_symbols: {r:?}"
    );
}

#[test]
fn outline_most_central_symbols_shape() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = get_repo_outline(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let central = r["data"]["most_central_symbols"].as_array().unwrap();
    for sym in central {
        assert!(sym["path"].is_string(), "should have path: {sym:?}");
        assert!(sym["score"].is_number(), "should have score: {sym:?}");
        assert_eq!(
            sym["kind"].as_str(),
            Some("file"),
            "centrality nodes are files: {sym:?}"
        );
        assert!(
            sym.get("symbol_id").is_none(),
            "symbol_id is no longer emitted (was file path, not a symbol): {sym:?}"
        );
    }
}

#[test]
fn outline_most_central_top_is_most_imported_file() {
    let (_ws, svc, sid) = build_sort_by_repo();
    let r = get_repo_outline(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let central = r["data"]["most_central_symbols"].as_array().unwrap();
    assert!(!central.is_empty(), "should have central symbols");
    let top_path = central[0]["path"].as_str().unwrap();
    assert!(
        top_path.contains("core.py"),
        "top central file should be core.py (imported by 2 files), got '{top_path}'"
    );
}
