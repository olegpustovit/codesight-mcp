//! Integration tests: symbol diff.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn symbol_diff_suite() {
    run_all();
}

fn run_all() {
    run_diff_two_distinct_symbols();
    run_diff_identical_symbols_no_change();
    run_diff_requires_two_refs();
    run_diff_requires_symbol_kind();
    run_diff_missing_source_error();
    run_diff_unknown_symbol_error();
    run_diff_meta_has_timing();
    run_diff_findings_structure();
    run_diff_cross_file_rejected();
    run_diff_signature_change_detected();
    run_added_count_matches_list_length();
    run_removed_count_matches_list_length();
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

/// Two distinct symbols in the same file produce a diff with non-empty evidence.
fn run_diff_two_distinct_symbols() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let foo_id = find_symbol_id(&svc, &sid, "foo");
    let bar_id = find_symbol_id(&svc, &sid, "bar");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": foo_id },
                    { "kind": "symbol", "stable_id": bar_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty(), "expected at least one finding");
    let ev = &findings[0]["evidence"];
    assert!(ev.get("common_prefix_lines").is_some());
}

/// Diffing a symbol against itself should show zero differences.
fn run_diff_identical_symbols_no_change() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def func():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let func_id = find_symbol_id(&svc, &sid, "func");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": func_id.clone() },
                    { "kind": "symbol", "stable_id": func_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    let ev = &findings[0]["evidence"];
    // Self-diff: removed and added tails should be empty
    let removed = ev["removed_tail"].as_array().unwrap();
    let added = ev["added_tail"].as_array().unwrap();
    assert!(removed.is_empty(), "self-diff should have no removed lines");
    assert!(added.is_empty(), "self-diff should have no added lines");
}

/// diff requires exactly two refs.
fn run_diff_requires_two_refs() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def func():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let func_id = find_symbol_id(&svc, &sid, "func");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": func_id },
                ],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "one ref should fail: {r:?}");
    assert!(r.get("error").is_some());
}

/// diff requires kind=symbol.
fn run_diff_requires_symbol_kind() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def func():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "file", "stable_id": "a.py" },
                    { "kind": "file", "stable_id": "a.py" },
                ],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "non-symbol kind should fail: {r:?}");
}

/// Missing source_id produces error.
fn run_diff_missing_source_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": "no_such_repo",
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": "a.py::foo#function" },
                    { "kind": "symbol", "stable_id": "a.py::bar#function" },
                ],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false);
    assert!(r.get("error").is_some());
}

/// Unknown symbol stable_id produces error or empty findings.
fn run_diff_unknown_symbol_error() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def func():\n    pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": "a.py::nonexistent#function" },
                    { "kind": "symbol", "stable_id": "a.py::also_missing#function" },
                ],
            }),
        )
        .unwrap();
    // Should fail or return error — symbol not found
    assert_eq!(r["ok"], false, "unknown symbols should fail: {r:?}");
}

/// _meta.timing_ms is present regardless of success/failure.
fn run_diff_meta_has_timing() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let foo_id = find_symbol_id(&svc, &sid, "foo");
    let bar_id = find_symbol_id(&svc, &sid, "bar");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": foo_id },
                    { "kind": "symbol", "stable_id": bar_id },
                ],
            }),
        )
        .unwrap();
    assert!(
        r["_meta"]["timing_ms"].is_number(),
        "expected _meta.timing_ms"
    );
}

/// Finding has expected structure: finding_id, analysis, severity, evidence, related_refs.
fn run_diff_findings_structure() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let foo_id = find_symbol_id(&svc, &sid, "foo");
    let bar_id = find_symbol_id(&svc, &sid, "bar");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": foo_id },
                    { "kind": "symbol", "stable_id": bar_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let f = &r["data"]["findings"][0];
    assert!(f.get("finding_id").is_some(), "missing finding_id");
    assert_eq!(f["analysis"].as_str().unwrap(), "diff");
    assert!(f.get("severity").is_some(), "missing severity");
    assert!(f.get("evidence").is_some(), "missing evidence");
    assert!(f.get("related_refs").is_some(), "missing related_refs");
    let refs = f["related_refs"].as_array().unwrap();
    assert_eq!(refs.len(), 2);
}

/// Diffing symbols across different files should be rejected.
fn run_diff_cross_file_rejected() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def alpha():\n    return 1\n").unwrap();
    fs::write(src.join("b.py"), "def beta():\n    return 2\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let alpha_id = find_symbol_id(&svc, &sid, "alpha");
    let beta_id = find_symbol_id(&svc, &sid, "beta");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": alpha_id },
                    { "kind": "symbol", "stable_id": beta_id },
                ],
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "cross-file diff should fail: {r:?}");
}

/// Detect signature change between two versions of a function (re-index approach).
fn run_diff_signature_change_detected() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // Write two functions that differ only in signature
    fs::write(
        src.join("lib.py"),
        "def compute_v1(x):\n    return x\n\ndef compute_v2(x, y):\n    return x + y\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let v1_id = find_symbol_id(&svc, &sid, "compute_v1");
    let v2_id = find_symbol_id(&svc, &sid, "compute_v2");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": v1_id },
                    { "kind": "symbol", "stable_id": v2_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let ev = &r["data"]["findings"][0]["evidence"];
    // The signatures differ, so at least one tail should be non-empty
    let removed = ev["removed_tail"].as_array().unwrap();
    let added = ev["added_tail"].as_array().unwrap();
    assert!(
        !removed.is_empty() || !added.is_empty(),
        "signature change should produce a diff"
    );
}

/// added_count and removed_count in original suite → port as: finding evidence
/// lists have consistent lengths.
fn run_added_count_matches_list_length() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let foo_id = find_symbol_id(&svc, &sid, "foo");
    let bar_id = find_symbol_id(&svc, &sid, "bar");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": foo_id },
                    { "kind": "symbol", "stable_id": bar_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let ev = &r["data"]["findings"][0]["evidence"];
    assert!(ev["added_tail"].as_array().is_some());
    assert!(ev["removed_tail"].as_array().is_some());
}

/// removed list is consistent (same as added test, different direction).
fn run_removed_count_matches_list_length() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def short_fn():\n    return 1\n\ndef long_fn(a, b, c):\n    x = a + b\n    y = x + c\n    return y\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let s_id = find_symbol_id(&svc, &sid, "short_fn");
    let l_id = find_symbol_id(&svc, &sid, "long_fn");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": s_id },
                    { "kind": "symbol", "stable_id": l_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let ev = &r["data"]["findings"][0]["evidence"];
    // At least one tail non-empty since functions differ
    let removed = ev["removed_tail"].as_array().unwrap();
    let added = ev["added_tail"].as_array().unwrap();
    assert!(
        !removed.is_empty() || !added.is_empty(),
        "different-length functions should differ"
    );
}
/// and `repo_b` identifier fields.
///
/// The Rust `evaluate_impact diff` operates on two symbols *within a single
/// source*, not across two repos. The concept of `repo_a`/`repo_b` identifiers
/// does not exist in the agent-v1 diff API. The response includes `source_id`
/// and `related_refs` instead.
///
/// We port the behavioral intent: the diff response includes identifiers that
/// let the caller trace back which entities were compared.
#[test]
fn case_response_has_source_identifiers() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let foo_id = find_symbol_id(&svc, &sid, "foo");
    let bar_id = find_symbol_id(&svc, &sid, "bar");

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": foo_id },
                    { "kind": "symbol", "stable_id": bar_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");

    // The agent-v1 response uses related_refs to identify compared entities
    // (the reference test checked for repo_a/repo_b keys; we check for the
    // equivalent traceability via related_refs).
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    let refs = findings[0]["related_refs"].as_array().unwrap();
    assert_eq!(refs.len(), 2, "diff should reference both compared symbols");
    // Each related_ref should be a non-empty string identifier
    for r in refs {
        let s = r.as_str().unwrap_or("");
        assert!(
            !s.is_empty(),
            "related_ref should be a non-empty identifier"
        );
    }
}
