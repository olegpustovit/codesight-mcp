//! Slice tests for dead-code analysis, plus a small MCP-envelope contract check.

use codesight_mcp::analysis::dead_code::dead_code_payload;
use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::IndexStore;
use codesight_mcp::test_support::{index_code_fixture, open_code_store, write_files};
use serde_json::{Value, json};
use tempfile::tempdir;

fn make_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(&src, files).unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

fn find_dead_code(
    store: &IndexStore,
    granularity: &str,
    min_confidence: f64,
    include_tests: bool,
    entry_patterns: &[String],
) -> Value {
    dead_code_payload(
        store,
        granularity,
        min_confidence,
        include_tests,
        entry_patterns,
        None,
    )
    .unwrap()
}

#[test]
fn dead_files_unreferenced_file_is_dead() {
    let (_ws, _config, _sid, store) = make_repo(&[
        (
            "main.py",
            "from src import core\nif __name__ == '__main__': core.run()",
        ),
        ("src/__init__.py", ""),
        ("src/core.py", "def run(): pass"),
        ("src/unused.py", "def helper(): pass"),
    ]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(dead.iter().any(|f| f.contains("unused")), "{dead:?}");
}

#[test]
fn dead_files_main_py_not_dead() {
    let (_ws, _config, _sid, store) = make_repo(&[("main.py", "def main(): pass")]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("main.py")));
}

#[test]
fn dead_files_init_file_not_dead() {
    let (_ws, _config, _sid, store) = make_repo(&[
        ("pkg/__init__.py", "from .core import run"),
        ("pkg/core.py", "def run(): pass"),
    ]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("__init__.py")));
}

#[test]
fn dead_files_main_guard_file_not_dead() {
    let (_ws, _config, _sid, store) = make_repo(&[(
        "runner.py",
        "def go(): pass\nif __name__ == '__main__': go()",
    )]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("runner.py")));
}

#[test]
fn dead_files_live_imported_file_not_dead() {
    let (_ws, _config, _sid, store) = make_repo(&[
        (
            "main.py",
            "from utils import helper\nif __name__ == '__main__': helper()",
        ),
        ("utils.py", "def helper(): pass"),
    ]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("utils")));
}

#[test]
fn dead_files_min_confidence_1_excludes_cascade() {
    let (_ws, _config, _sid, store) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("orphan_a.py", "from orphan_b import x"),
        ("orphan_b.py", "x = 1"),
    ]);
    let r_low = find_dead_code(&store, "file", 0.7, true, &[]);
    let r_high = find_dead_code(&store, "file", 1.0, true, &[]);
    assert!(
        r_high["dead_file_count"].as_u64().unwrap() <= r_low["dead_file_count"].as_u64().unwrap()
    );
}

#[test]
fn dead_symbols_in_dead_file_is_reported() {
    let (_ws, _config, _sid, store) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("unused.py", "def forgotten(): pass\nclass OldClass: pass"),
    ]);
    let r = find_dead_code(&store, "symbol", 0.5, true, &[]);
    let ids: Vec<&str> = r["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["symbol_id"].as_str())
        .collect();
    assert!(ids.iter().any(|id| id.contains("forgotten")), "{ids:?}");
}

#[test]
fn dead_symbols_in_live_file_not_reported() {
    let (_ws, _config, _sid, store) = make_repo(&[
        (
            "main.py",
            "from lib import used_fn\nif __name__ == '__main__': used_fn()",
        ),
        ("lib.py", "def used_fn(): pass"),
    ]);
    let r = find_dead_code(&store, "symbol", 0.5, true, &[]);
    let ids: Vec<&str> = r["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["symbol_id"].as_str())
        .collect();
    assert!(!ids.iter().any(|id| id.contains("used_fn")));
}

#[test]
fn dead_symbols_granularity_file_omits_symbols() {
    let (_ws, _config, _sid, store) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("dead.py", "def noop(): pass"),
    ]);
    let r = find_dead_code(&store, "file", 0.5, true, &[]);
    assert_eq!(r["dead_symbols"].as_array().unwrap().len(), 0);
    assert_eq!(r["dead_symbol_count"].as_u64().unwrap(), 0);
}

#[test]
fn options_include_tests_false_excludes_test_files() {
    let (_ws, _config, _sid, store) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("tests/test_foo.py", "def test_something(): assert True"),
    ]);
    let r = find_dead_code(&store, "file", 0.5, false, &[]);
    let dead: Vec<&str> = r["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("test_foo")));
}

#[test]
fn options_entry_point_patterns_respected() {
    let (_ws, _config, _sid, store) = make_repo(&[("scripts/deploy.py", "def deploy(): pass")]);
    let r_without = find_dead_code(&store, "file", 0.5, true, &[]);
    let r_with = find_dead_code(&store, "file", 0.5, true, &["scripts/*.py".to_string()]);
    let dead_without: Vec<&str> = r_without["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    let dead_with: Vec<&str> = r_with["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(dead_without.iter().any(|f| f.contains("deploy")));
    assert!(!dead_with.iter().any(|f| f.contains("deploy")));
}

#[test]
fn errors_unknown_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "nonexistent/repo",
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn errors_response_shape() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(&src, &[("main.py", "if __name__ == '__main__': pass")]).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
                "granularity": "file",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let data = &r["data"];
    for key in [
        "dead_symbols",
        "dead_files",
        "dead_file_count",
        "dead_symbol_count",
        "live_root_count",
        "analysis_notes",
    ] {
        assert!(!data[key].is_null(), "missing key: {key}");
    }
    assert!(r["_meta"].is_object());
}
