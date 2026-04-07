//! Integration tests: find dead code.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
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

fn make_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    for (rel, content) in files {
        let p = src.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn find_dead_code(svc: &CodeService, sid: &str, granularity: &str) -> Value {
    svc.dispatch(
        "evaluate_quality",
        json!({
            "source_id": sid,
            "analysis": "dead_code",
            "granularity": granularity,
        }),
    )
    .unwrap()
}

fn find_dead_code_with_opts(svc: &CodeService, sid: &str, extra: Value) -> Value {
    let mut args = json!({
        "source_id": sid,
        "analysis": "dead_code",
    });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            args[k] = v;
        }
    }
    svc.dispatch("evaluate_quality", args).unwrap()
}

// ── TestDeadFiles: basic dead file detection ─────────────────────────────────

#[test]
fn dead_files_unreferenced_file_is_dead() {
    let (_ws, svc, sid) = make_repo(&[
        (
            "main.py",
            "from src import core\nif __name__ == '__main__': core.run()",
        ),
        ("src/__init__.py", ""),
        ("src/core.py", "def run(): pass"),
        ("src/unused.py", "def helper(): pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(
        dead.iter().any(|f| f.contains("unused")),
        "Expected unused.py in dead files, got {dead:?}"
    );
}

#[test]
fn dead_files_main_py_not_dead() {
    let (_ws, svc, sid) = make_repo(&[("main.py", "def main(): pass")]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("main.py")));
}

#[test]
fn dead_files_init_file_not_dead() {
    let (_ws, svc, sid) = make_repo(&[
        ("pkg/__init__.py", "from .core import run"),
        ("pkg/core.py", "def run(): pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("__init__.py")));
}

#[test]
fn dead_files_main_guard_file_not_dead() {
    let (_ws, svc, sid) = make_repo(&[(
        "runner.py",
        "def go(): pass\nif __name__ == '__main__': go()",
    )]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("runner.py")));
}

#[test]
fn dead_files_live_imported_file_not_dead() {
    let (_ws, svc, sid) = make_repo(&[
        (
            "main.py",
            "from utils import helper\nif __name__ == '__main__': helper()",
        ),
        ("utils.py", "def helper(): pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("utils")));
}

#[test]
fn dead_files_min_confidence_1_excludes_cascade() {
    let (_ws, svc, sid) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("orphan_a.py", "from orphan_b import x"),
        ("orphan_b.py", "x = 1"),
    ]);
    let r_low = find_dead_code_with_opts(
        &svc,
        &sid,
        json!({ "granularity": "file", "min_confidence": 0.7 }),
    );
    let r_high = find_dead_code_with_opts(
        &svc,
        &sid,
        json!({ "granularity": "file", "min_confidence": 1.0 }),
    );
    assert!(
        r_high["data"]["dead_file_count"].as_u64().unwrap()
            <= r_low["data"]["dead_file_count"].as_u64().unwrap()
    );
}

// ── TestDeadSymbols: symbol granularity ──────────────────────────────────────

#[test]
fn dead_symbols_in_dead_file_is_reported() {
    let (_ws, svc, sid) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("unused.py", "def forgotten(): pass\nclass OldClass: pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "symbol");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let ids: Vec<&str> = r["data"]["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["symbol_id"].as_str())
        .collect();
    assert!(
        ids.iter().any(|id| id.contains("forgotten")),
        "Expected forgotten in {ids:?}"
    );
}

#[test]
fn dead_symbols_in_live_file_not_reported() {
    let (_ws, svc, sid) = make_repo(&[
        (
            "main.py",
            "from lib import used_fn\nif __name__ == '__main__': used_fn()",
        ),
        ("lib.py", "def used_fn(): pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "symbol");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let ids: Vec<&str> = r["data"]["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["symbol_id"].as_str())
        .collect();
    assert!(!ids.iter().any(|id| id.contains("used_fn")));
}

#[test]
fn dead_symbols_granularity_file_omits_symbols() {
    let (_ws, svc, sid) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("dead.py", "def noop(): pass"),
    ]);
    let r = find_dead_code(&svc, &sid, "file");
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert_eq!(r["data"]["dead_symbols"].as_array().unwrap().len(), 0);
    assert_eq!(r["data"]["dead_symbol_count"].as_u64().unwrap(), 0);
}

// ── TestOptions: include_tests + entry_point_patterns ────────────────────────

#[test]
fn options_include_tests_false_excludes_test_files() {
    let (_ws, svc, sid) = make_repo(&[
        ("main.py", "if __name__ == '__main__': pass"),
        ("tests/test_foo.py", "def test_something(): assert True"),
    ]);
    let r = find_dead_code_with_opts(
        &svc,
        &sid,
        json!({ "granularity": "file", "include_tests": false }),
    );
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let dead: Vec<&str> = r["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(!dead.iter().any(|f| f.contains("test_foo")));
}

#[test]
fn options_entry_point_patterns_respected() {
    let (_ws, svc, sid) = make_repo(&[("scripts/deploy.py", "def deploy(): pass")]);
    let r_without = find_dead_code(&svc, &sid, "file");
    let r_with = find_dead_code_with_opts(
        &svc,
        &sid,
        json!({ "granularity": "file", "entry_point_patterns": ["scripts/*.py"] }),
    );
    let dead_without: Vec<&str> = r_without["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    let dead_with: Vec<&str> = r_with["data"]["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["file"].as_str())
        .collect();
    assert!(dead_without.iter().any(|f| f.contains("deploy")));
    assert!(!dead_with.iter().any(|f| f.contains("deploy")));
}

// ── TestErrors: error cases + response shape ─────────────────────────────────

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
    let (_ws, svc, sid) = make_repo(&[("main.py", "if __name__ == '__main__': pass")]);
    let r = find_dead_code(&svc, &sid, "file");
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
        assert!(!data[key].is_null(), "Missing key: {key}");
    }
    assert!(r["_meta"].is_object());
}
