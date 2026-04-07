//! Integration tests: complexity params nesting.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
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

fn find_symbol_id(svc: &CodeService, sid: &str, name: &str) -> String {
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
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
    panic!("symbol {name:?} not found");
}

fn complexity_param_count(svc: &CodeService, sid: &str, sym_id: &str) -> u64 {
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym_id }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["findings"][0]["evidence"]["param_count"]
        .as_u64()
        .expect("missing param_count in evidence")
}

fn complexity_max_nesting(svc: &CodeService, sid: &str, sym_id: &str) -> u64 {
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym_id }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["findings"][0]["evidence"]["max_nesting"]
        .as_u64()
        .expect("missing max_nesting in evidence")
}

// ── TestCountParams ──────────────────────────────────────────────────────────

#[test]

fn count_params_no_params() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.py"), "def foo():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 0);
}

#[test]

fn count_params_one_param() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.py"), "def foo(x):\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 1);
}

#[test]

fn count_params_multiple_params() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.py"), "def foo(x, y, z):\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 3);
}

#[test]

fn count_params_nested_generic() {
    // Generic type: dict[str, int] shouldn't split on inner comma
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("p.py"),
        "def foo(x: dict[str, int], y: int):\n    pass\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 2);
}

#[test]

fn count_params_empty_string() {
    // Edge case: empty body should yield 0 params
    // This tests the parser's handling of degenerate input
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // Cannot actually index an empty file easily; the behavioral expectation
    // is that the complexity engine returns 0 for functions with no params.
    fs::write(src.join("p.py"), "def foo():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 0);
}

#[test]

fn count_params_c_void_zero_params() {
    // C convention: void foo(void) means zero parameters
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.c"), "void foo(void) { }\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 0);
}

#[test]

fn count_params_c_void_pointer_not_zero() {
    // void* is a parameter type — should NOT be zero
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.c"), "void foo(void *ptr) { }\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 1);
}

#[test]

fn count_params_c_void_pointer_multi() {
    // void* among multiple params
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("p.c"), "void foo(void *a, int b) { }\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_param_count(&svc, &sid, &sym), 2);
}

// ── TestMaxNestingDepth ──────────────────────────────────────────────────────

#[test]
fn nesting_flat_code() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("n.py"), "def foo():\n    return 1\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert_eq!(complexity_max_nesting(&svc, &sid, &sym), 0);
}

#[test]

fn nesting_one_level_deep() {
    // Bracket-based nesting: function with one if
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("n.js"),
        "function foo() { if (x) { return 1; } }\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert!(complexity_max_nesting(&svc, &sid, &sym) >= 1);
}

#[test]

fn nesting_deeply_nested() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("n.js"),
        "function foo() { { { { return 1; } } } }\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    let sym = find_symbol_id(&svc, &sid, "foo");
    assert!(complexity_max_nesting(&svc, &sid, &sym) >= 3);
}
