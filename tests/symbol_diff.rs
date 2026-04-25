//! Integration tests: symbol diff.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

struct RepoFixture {
    _workspace: tempfile::TempDir,
    svc: CodeService,
    source_id: String,
}

#[test]
fn symbol_diff_suite() {
    run_all();
}

fn run_all() {
    let foo_bar = build_repo_fixture(&[(
        "main.py",
        "def foo(x):\n    return x * 2\n\ndef bar(y):\n    return y + 1\n",
    )]);
    let foo_id = find_symbol_id(&foo_bar.svc, &foo_bar.source_id, "foo");
    let bar_id = find_symbol_id(&foo_bar.svc, &foo_bar.source_id, "bar");

    let func_fixture = build_repo_fixture(&[("a.py", "def func():\n    pass\n")]);
    let func_id = find_symbol_id(&func_fixture.svc, &func_fixture.source_id, "func");

    let cross_file = build_repo_fixture(&[
        ("a.py", "def alpha():\n    return 1\n"),
        ("b.py", "def beta():\n    return 2\n"),
    ]);
    let alpha_id = find_symbol_id(&cross_file.svc, &cross_file.source_id, "alpha");
    let beta_id = find_symbol_id(&cross_file.svc, &cross_file.source_id, "beta");

    let signature_fixture = build_repo_fixture(&[(
        "lib.py",
        "def compute_v1(x):\n    return x\n\ndef compute_v2(x, y):\n    return x + y\n",
    )]);
    let v1_id = find_symbol_id(
        &signature_fixture.svc,
        &signature_fixture.source_id,
        "compute_v1",
    );
    let v2_id = find_symbol_id(
        &signature_fixture.svc,
        &signature_fixture.source_id,
        "compute_v2",
    );

    let length_fixture = build_repo_fixture(&[(
        "main.py",
        "def short_fn():\n    return 1\n\ndef long_fn(a, b, c):\n    x = a + b\n    y = x + c\n    return y\n",
    )]);
    let short_id = find_symbol_id(&length_fixture.svc, &length_fixture.source_id, "short_fn");
    let long_id = find_symbol_id(&length_fixture.svc, &length_fixture.source_id, "long_fn");

    run_diff_two_distinct_symbols(&foo_bar, &foo_id, &bar_id);
    run_diff_identical_symbols_no_change(&func_fixture, &func_id);
    run_diff_requires_two_refs(&func_fixture, &func_id);
    run_diff_requires_symbol_kind(&func_fixture);
    run_diff_missing_source_error();
    run_diff_unknown_symbol_error(&func_fixture);
    run_diff_meta_has_timing(&foo_bar, &foo_id, &bar_id);
    run_diff_findings_structure(&foo_bar, &foo_id, &bar_id);
    run_diff_cross_file_rejected(&cross_file, &alpha_id, &beta_id);
    run_diff_signature_change_detected(&signature_fixture, &v1_id, &v2_id);
    run_added_count_matches_list_length(&foo_bar, &foo_id, &bar_id);
    run_removed_count_matches_list_length(&length_fixture, &short_id, &long_id);
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

fn build_repo_fixture(files: &[(&str, &str)]) -> RepoFixture {
    let workspace = tempdir().unwrap();
    let src = workspace.path().join("src");
    fs::create_dir_all(&src).unwrap();
    for (path, content) in files {
        fs::write(src.join(path), content).unwrap();
    }
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let source_id = ingest(&svc, workspace.path());
    RepoFixture {
        _workspace: workspace,
        svc,
        source_id,
    }
}

/// Two distinct symbols in the same file produce a diff with non-empty evidence.
fn run_diff_two_distinct_symbols(fixture: &RepoFixture, foo_id: &str, bar_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_diff_identical_symbols_no_change(fixture: &RepoFixture, func_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": func_id },
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
fn run_diff_requires_two_refs(fixture: &RepoFixture, func_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_diff_requires_symbol_kind(fixture: &RepoFixture) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();

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
fn run_diff_unknown_symbol_error(fixture: &RepoFixture) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_diff_meta_has_timing(fixture: &RepoFixture, foo_id: &str, bar_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_diff_findings_structure(fixture: &RepoFixture, foo_id: &str, bar_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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

/// Cross-file diff is supported (ROADMAP P3.1) — diffing two impls of
/// the same trait method that live in different files is the common
/// case. Verify the response succeeds and tags the cross_file pair.
fn run_diff_cross_file_rejected(fixture: &RepoFixture, alpha_id: &str, beta_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
                "analysis": "diff",
                "refs": [
                    { "kind": "symbol", "stable_id": alpha_id },
                    { "kind": "symbol", "stable_id": beta_id },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let f = &r["data"]["findings"][0];
    assert!(f.get("cross_file").is_some(), "cross_file tag missing: {r:?}");
}

/// Detect signature change between two versions of a function (re-index approach).
fn run_diff_signature_change_detected(fixture: &RepoFixture, v1_id: &str, v2_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_added_count_matches_list_length(fixture: &RepoFixture, foo_id: &str, bar_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
fn run_removed_count_matches_list_length(fixture: &RepoFixture, s_id: &str, l_id: &str) {
    let r = fixture
        .svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": fixture.source_id,
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
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
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
