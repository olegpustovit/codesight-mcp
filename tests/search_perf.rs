//! Integration tests: search perf.

use std::fs;
use std::time::Instant;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the same 5-file synthetic repo from the reference test.
///
/// Returns the repo root (the `src` subdir). Callers must ingest this path
/// rather than the shared tempdir — otherwise codesight's own index dir
/// (a sibling of `src` under the tempdir) gets picked up by the walker and
/// session-ledger JSON keys leak into the symbol set between calls.
fn build_search_perf_repo(parent: &std::path::Path) -> std::path::PathBuf {
    let src = parent.join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("parser.py"),
        "def parse_file(path):\n    return {}\n\n\
         def parse_tokens(source):\n    return []\n\n\
         def parse_imports(source):\n    return []\n\n\
         class ParseError(Exception):\n    pass\n\n\
         class TokenStream:\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("search.py"),
        "def search_symbols(query, repo):\n    return []\n\n\
         def search_text(query, repo):\n    return []\n\n\
         def rank_results(results, query):\n    return results\n\n\
         class SearchIndex:\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("storage.py"),
        "def save_index(index, path):\n    pass\n\n\
         def load_index(path):\n    return None\n\n\
         def clear_cache():\n    pass\n\n\
         class CodeIndex:\n    pass\n\n\
         class IndexStore:\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("server.py"),
        "def run_server(host, port):\n    pass\n\n\
         def handle_request(req):\n    return {}\n\n\
         def dispatch_tool(name, args):\n    return {}\n",
    )
    .unwrap();
    fs::write(
        src.join("utils.py"),
        "MAX_RESULTS = 500\n\n\
         def tokenize(text):\n    return text.split()\n\n\
         def normalize(name):\n    return name.lower()\n\n\
         def resolve_path(path, root):\n    return path\n",
    )
    .unwrap();

    src
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
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn search(svc: &CodeService, source_id: &str, query: &str, limit: usize) -> Value {
    svc.dispatch(
        "search_catalog",
        json!({
            "source_id": source_id,
            "query": query,
            "domain": "code",
            "unit_kinds": ["symbol"],
            "limit": limit,
        }),
    )
    .unwrap()
}

fn semantic_search(svc: &CodeService, source_id: &str, query: &str, limit: usize) -> Value {
    svc.dispatch(
        "search_catalog",
        json!({
            "source_id": source_id,
            "query": query,
            "domain": "code",
            "unit_kinds": ["symbol"],
            "semantic": true,
            "limit": limit,
        }),
    )
    .unwrap()
}

fn symbol_names(data: &Value) -> Vec<String> {
    data["data"]["items"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|it| it.get("ref")?.get("title")?.as_str().map(String::from))
        .collect()
}

// ---------------------------------------------------------------------------
// Shared setup — single workspace for all tests in this module
// ---------------------------------------------------------------------------

struct PerfRepo {
    _ws: tempfile::TempDir,
    idx_root: std::path::PathBuf,
    svc: CodeService,
    source_id: String,
}

fn setup_perf_repo() -> PerfRepo {
    let ws = tempdir().unwrap();
    let root = build_search_perf_repo(ws.path());
    let idx_root = ws.path().join("idx");
    let svc = CodeService::new(AppConfig::new(idx_root.clone())).unwrap();
    let source_id = ingest(&svc, &root);
    PerfRepo {
        _ws: ws,
        idx_root,
        svc,
        source_id,
    }
}

impl PerfRepo {
    fn fresh_service(&self) -> CodeService {
        CodeService::new(AppConfig::new(self.idx_root.clone())).unwrap()
    }
}

const QUERIES: &[&str] = &["parse", "search", "index", "token", "server", "load"];

// ---------------------------------------------------------------------------
// T14 -- Correctness: result stability across calls
// ---------------------------------------------------------------------------

#[test]
fn search_perf_suite() {
    let repo = setup_perf_repo();
    run_all(&repo);
}

fn run_all(repo: &PerfRepo) {
    case_results_stable_across_two_calls(repo);
    case_relevant_symbol_appears_in_results(repo);
    case_query_returns_nonempty_results(repo);
    case_semantic_query_returns_nonempty_results(repo);
    case_cold_search_within_budget(repo);
    case_warm_search_within_budget(repo);
    case_warm_semantic_search_within_budget(repo);
    case_warm_faster_than_cold(repo);
    case_semantic_warm_faster_than_cold(repo);
    case_timing_meta_reported(repo);
}

/// BM25 results must be deterministic: identical on repeated calls.
fn case_results_stable_across_two_calls(p: &PerfRepo) {
    for q in QUERIES {
        let r1 = search(&p.svc, &p.source_id, q, 10);
        let r2 = search(&p.svc, &p.source_id, q, 10);
        assert!(r1["ok"].as_bool().unwrap(), "Query '{q}': {r1:?}");
        let ids1 = symbol_names(&r1);
        let ids2 = symbol_names(&r2);
        assert_eq!(
            ids1, ids2,
            "Query '{q}': result order differs between calls"
        );
    }
}

/// Querying "parse" should surface parse_file / parse_tokens / parse_imports.
fn case_relevant_symbol_appears_in_results(p: &PerfRepo) {
    let r = search(&p.svc, &p.source_id, "parse", 5);
    assert!(r["ok"].as_bool().unwrap());
    let names = symbol_names(&r);
    let parse_funcs: Vec<&str> = names
        .iter()
        .filter(|n| n.starts_with("parse_") || n.starts_with("Parse"))
        .map(String::as_str)
        .collect();
    assert!(
        !parse_funcs.is_empty(),
        "Expected at least one parse_* symbol in top-5, got: {names:?}"
    );
}

/// All test queries should return at least one result.
fn case_query_returns_nonempty_results(p: &PerfRepo) {
    for q in QUERIES {
        let r = search(&p.svc, &p.source_id, q, 10);
        assert!(r["ok"].as_bool().unwrap(), "Query '{q}' errored: {r:?}");
        let items = r["data"]["items"].as_array().unwrap();
        assert!(!items.is_empty(), "Query '{q}' returned no results");
    }
}

/// Semantic queries should return relevant symbol hits on the shared fixture.
fn case_semantic_query_returns_nonempty_results(p: &PerfRepo) {
    let r = semantic_search(&p.svc, &p.source_id, "function that parses files", 10);
    assert!(r["ok"].as_bool().unwrap(), "semantic query errored: {r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "semantic query returned no results");
}

// ---------------------------------------------------------------------------
// T14 -- Latency budgets (in-process benchmarks)
// ---------------------------------------------------------------------------

/// Cold search (fresh service, no cached state) must complete within 2000 ms.
fn case_cold_search_within_budget(p: &PerfRepo) {
    let svc2 = p.fresh_service();
    let start = Instant::now();
    let r = search(&svc2, &p.source_id, "parse", 10);
    let elapsed_ms = start.elapsed().as_millis();

    assert!(r["ok"].as_bool().unwrap());
    assert!(
        elapsed_ms < 2000,
        "Cold search took {elapsed_ms}ms -- exceeds 2000ms budget"
    );
}

/// Warm search (cache populated) must complete within 500 ms.
fn case_warm_search_within_budget(p: &PerfRepo) {
    // Warm up
    let _ = search(&p.svc, &p.source_id, "parse", 10);

    let start = Instant::now();
    let r = search(&p.svc, &p.source_id, "parse", 10);
    let elapsed_ms = start.elapsed().as_millis();

    assert!(r["ok"].as_bool().unwrap());
    assert!(
        elapsed_ms < 500,
        "Warm search took {elapsed_ms}ms -- exceeds 500ms budget"
    );
}

/// Warm semantic search on the already-indexed fixture should complete
/// within 1000 ms. This includes query embedding plus sqlite-vec KNN.
fn case_warm_semantic_search_within_budget(p: &PerfRepo) {
    let _ = semantic_search(&p.svc, &p.source_id, "function that parses files", 10);

    let start = Instant::now();
    let r = semantic_search(&p.svc, &p.source_id, "function that parses files", 10);
    let elapsed_ms = start.elapsed().as_millis();

    assert!(r["ok"].as_bool().unwrap());
    assert!(
        elapsed_ms < 1000,
        "Warm semantic search took {elapsed_ms}ms -- exceeds 1000ms budget"
    );
}

/// Warm search should not be significantly slower than cold search.
fn case_warm_faster_than_cold(p: &PerfRepo) {
    let svc2 = p.fresh_service();
    let start = Instant::now();
    let _ = search(&svc2, &p.source_id, "search", 10);
    let cold_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let _ = search(&svc2, &p.source_id, "search", 10);
    let warm_ms = start.elapsed().as_millis();

    // On very fast machines cold/warm may be within noise -- only enforce
    // that warm is not significantly slower (10x) than cold.
    assert!(
        warm_ms < cold_ms * 10 || warm_ms < 500,
        "Warm ({warm_ms}ms) unexpectedly much slower than cold ({cold_ms}ms)"
    );
}

/// Warm semantic search should not be materially slower than the cold
/// semantic path once the embedder is initialized.
fn case_semantic_warm_faster_than_cold(p: &PerfRepo) {
    let svc2 = p.fresh_service();
    let start = Instant::now();
    let _ = semantic_search(&svc2, &p.source_id, "function that parses files", 10);
    let cold_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let _ = semantic_search(&svc2, &p.source_id, "function that parses files", 10);
    let warm_ms = start.elapsed().as_millis();

    assert!(
        warm_ms < cold_ms * 2 || warm_ms < 1000,
        "Warm semantic ({warm_ms}ms) unexpectedly much slower than cold ({cold_ms}ms)"
    );
}

/// search_catalog _meta must include timing_ms.
fn case_timing_meta_reported(p: &PerfRepo) {
    let r = search(&p.svc, &p.source_id, "token", 5);
    assert!(r["ok"].as_bool().unwrap());
    assert!(r["_meta"].is_object(), "_meta missing from response");
    let timing = r["_meta"]["timing_ms"].as_f64();
    assert!(
        timing.is_some() && timing.unwrap() >= 0.0,
        "_meta.timing_ms missing or negative: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// The Python test uses debug=True to get per-result scores. The Rust
// search_catalog does not expose per-result BM25 scores, so we verify
// stability through ordering (covered by case_results_stable_across_two_calls).
// ---------------------------------------------------------------------------

#[test]
fn case_scores_stable_across_two_calls() {
    // BM25 scores must be identical on repeated calls.
    // The Rust search_catalog surfaces per-result `score` in every item.
    let ws = tempdir().unwrap();
    let src_root = build_search_perf_repo(ws.path());
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let source_id = ingest(&svc, &src_root);

    let r1 = search(&svc, &source_id, "parse file", 5);
    assert!(r1["ok"].as_bool().unwrap(), "search 1 failed: {r1:?}");

    let r2 = search(&svc, &source_id, "parse file", 5);
    assert!(r2["ok"].as_bool().unwrap(), "search 2 failed: {r2:?}");

    let scores1: Vec<f64> = r1["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["score"].as_f64().unwrap())
        .collect();

    let scores2: Vec<f64> = r2["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["score"].as_f64().unwrap())
        .collect();

    assert!(
        !scores1.is_empty(),
        "expected at least one search result with a score"
    );
    assert_eq!(
        scores1, scores2,
        "BM25 scores must be identical across repeated calls"
    );
}
