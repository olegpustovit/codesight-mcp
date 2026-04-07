//! Integration tests: complexity.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn complexity_suite() {
    run_all();
}

fn run_all() {
    run_simple_function_low_complexity();
    run_if_increases_branch_hints();
    run_multiple_branches_higher_hints();
    run_for_loop_increases_hints();
    run_while_loop_increases_hints();
    run_missing_source_error();
    run_unknown_symbol_error();
    run_meta_has_timing();
    run_findings_structure();
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
    panic!("symbol {name:?} not found in search results for {source_id}");
}

/// Simple function with no branches has zero or very low branch hints.
fn run_simple_function_low_complexity() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("simple.py"), "def foo():\n    return 42\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "foo");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let f = &r["data"]["findings"][0];
    let hints = f["evidence"]["branch_token_hints"].as_u64().unwrap();
    assert_eq!(hints, 0, "no branches → zero branch_token_hints");
    let lines = f["evidence"]["lines"].as_u64().unwrap();
    assert!(lines >= 1, "function has at least 1 line");
}

/// A function with an `if` statement should have branch_token_hints > 0.
fn run_if_increases_branch_hints() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("branchy.py"),
        "def branchy(x):\n    if x > 0:\n        return 1\n    return 0\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "branchy");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let hints = r["data"]["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    assert!(hints > 0, "if statement should produce branch hints");
}

/// Multiple branches produce more hints.
fn run_multiple_branches_higher_hints() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("multi.py"),
        concat!(
            "def multi(x, y):\n",
            "    if x:\n",
            "        if y:\n",
            "            return 1\n",
            "        else:\n",
            "            return -1\n",
            "    else:\n",
            "        return 0\n",
        ),
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "multi");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let hints = r["data"]["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    // At least 2 `if ` tokens; `else:` (Python) may not match `"else "` pattern
    assert!(
        hints >= 2,
        "multiple branches should produce ≥2 hints, got {hints}"
    );
}

/// for loop body with branching should produce branch hints.
fn run_for_loop_increases_hints() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // Use JS-style for loop that uses `if` keyword in the body
    fs::write(
        src.join("loop.js"),
        "function loopy() {\n    for (let i = 0; i < 10; i++) {\n        if (i > 5) { break; }\n    }\n}\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "loopy");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let hints = r["data"]["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    assert!(hints >= 1, "for+if should produce hints, got {hints}");
}

/// while loop with break has at least one branch-like structure.
fn run_while_loop_increases_hints() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("wh.py"),
        "def whiler():\n    while True:\n        if done:\n            break\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "whiler");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let hints = r["data"]["findings"][0]["evidence"]["branch_token_hints"]
        .as_u64()
        .unwrap();
    assert!(hints >= 1, "while+if should produce hints, got {hints}");
}

/// Missing source_id produces error.
fn run_missing_source_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": "a.py::foo#function" }],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false);
    assert!(r.get("error").is_some());
}

/// Unknown symbol produces error.
fn run_unknown_symbol_error() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def real():\n    pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": "a.py::ghost#function" }],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "unknown symbol should fail: {r:?}");
}

/// _meta.timing_ms is present.
fn run_meta_has_timing() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("t.py"), "def t():\n    return 1\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "t");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(
        r["_meta"]["timing_ms"].is_number(),
        "expected _meta.timing_ms"
    );
}

/// Finding has expected structure.
fn run_findings_structure() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("s.py"), "def s():\n    return 1\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let sym = find_symbol_id(&svc, &sid, "s");

    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let f = &r["data"]["findings"][0];
    assert!(f.get("finding_id").is_some(), "missing finding_id");
    assert_eq!(f["analysis"].as_str().unwrap(), "complexity");
    assert!(f.get("severity").is_some(), "missing severity");
    assert!(f.get("confidence").is_some(), "missing confidence");
    assert!(f.get("evidence").is_some(), "missing evidence");
    let ev = &f["evidence"];
    assert!(ev.get("lines").is_some(), "missing evidence.lines");
    assert!(
        ev.get("branch_token_hints").is_some(),
        "missing evidence.branch_token_hints"
    );
}
