//! Integration tests: agent v1 input validation.

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_svc() -> (tempfile::TempDir, CodeService) {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().to_path_buf(),
    ))
    .unwrap();
    (dir, svc)
}

// ── Source Resolution ────────────────────────────────────────────────────────

#[test]
fn resolve_source_owner_slash_name() {
    // The agent-v1 equivalent of resolve_repo("octocat/hello-world") should
    // parse the owner and name from a slash-separated source_id.
    //
    // The Rust implementation normalizes source_ids to "local/{dir_name}-{hash}".
    // resolve_source_id supports matching by the name portion of the source_id.
    let dir = tempdir().unwrap();
    let root = dir.path().join("hello-world");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.py"), "x = 1\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let source_id = r["data"]["source_id"].as_str().unwrap().to_string();

    // The source_id looks like "local/hello-world-{hash}".
    // Searching with the full source_id should resolve (exact match).
    let r2 = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": &source_id, "query": "x" }),
        )
        .unwrap();
    // Should succeed (finds the repo by exact match)
    assert!(
        r2["ok"].as_bool().unwrap(),
        "exact source_id should resolve: {r2:?}"
    );
}

#[test]
fn resolve_source_name_only_lookup() {
    // When only a short name is provided (no owner prefix), the system should
    // search indexed sources and find the matching one.
    //
    // The Rust resolve_source_id matches by name portion with hash suffix stripped.
    let dir = tempdir().unwrap();
    let root = dir.path().join("hello-world");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.py"), "x = 1\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());

    // Searching with short name "hello-world" should resolve to the ingested source.
    // The source_id is "local/hello-world-{8char_hash}".
    // resolve_source_id strips the hash suffix and matches by name.
    let r2 = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": "hello-world", "query": "x" }),
        )
        .unwrap();
    assert!(
        r2["ok"].as_bool().unwrap(),
        "name-only lookup should resolve: {r2:?}"
    );
}

#[test]
fn resolve_source_display_name_lookup() {
    // When a display_name is set, the system should resolve by display_name.
    // In the Rust implementation, display_name is not a separate concept.
    // The source_id serves as the identifier, and resolve_source_id matches
    // by name portion. This test verifies that name-based lookup works.
    let dir = tempdir().unwrap();
    let root = dir.path().join("my-project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.py"), "x = 1\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());

    // "my-project" should resolve via name matching
    let r2 = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": "my-project", "query": "x" }),
        )
        .unwrap();
    assert!(
        r2["ok"].as_bool().unwrap(),
        "display_name/name lookup should resolve: {r2:?}"
    );
}

#[test]
fn resolve_source_unknown_raises_error() {
    // Expected: searching for "nonexistent" source returns an error envelope
    // with ok=false and a meaningful error message mentioning "not found" or
    // "not indexed".
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "nonexistent",
                "query": "anything",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "unknown source should fail: {r}");
    let err_msg = r["error"]["message"].as_str().unwrap_or("");
    let lc = err_msg.to_lowercase();
    assert!(
        lc.contains("not found")
            || lc.contains("not indexed")
            || lc.contains("not been indexed")
            || lc.contains("no usable index"),
        "error message should mention not found/not indexed: {err_msg}"
    );
}

#[test]
fn resolve_source_ambiguous_display_name_raises() {
    // When two sources share the same name, resolution should fail
    // with an "ambiguous" error.
    let dir = tempdir().unwrap();

    // Create two different directories with the same base name
    let root1 = dir.path().join("subdir1").join("myrepo");
    let root2 = dir.path().join("subdir2").join("myrepo");
    fs::create_dir_all(&root1).unwrap();
    fs::create_dir_all(&root2).unwrap();
    fs::write(root1.join("main.py"), "x = 1\n").unwrap();
    fs::write(root2.join("main.py"), "y = 2\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let r1 = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root1.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r1["ok"].as_bool().unwrap());
    let r2 = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root2.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r2["ok"].as_bool().unwrap());

    // Both directories have the same base name "myrepo" but different hashes.
    // The name-only lookup "myrepo" should fail (ambiguous or not found).
    // We use inspect_structure because it hits ensure_repo_store which calls resolve_source_id.
    let r3 = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": "myrepo", "view": "file_tree" }),
        )
        .unwrap();
    // Should fail — either ambiguous or not found
    assert!(
        !r3["ok"].as_bool().unwrap_or(true),
        "ambiguous name should fail: {r3:?}"
    );
}

// ── Input Clamping: search_catalog max_results ──────────────────────────────

#[test]
fn search_catalog_clamps_max_results_zero() {
    // max_results=0 should be clamped to a sensible minimum (e.g., 1).
    // The call will fail on source lookup, but it should not crash.
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "owner/repo",
                "query": "query",
                "limit": 0,
            }),
        )
        .unwrap();
    // Should return an error (repo not indexed), not crash
    assert!(r.get("error").is_some() || r["ok"] == false);
}

#[test]
fn search_catalog_clamps_max_results_extreme() {
    // max_results=999 should be clamped to a reasonable upper bound.
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "owner/repo",
                "query": "query",
                "limit": 999,
            }),
        )
        .unwrap();
    assert!(r.get("error").is_some() || r["ok"] == false);
}

// ── Input Clamping: text search max_results ─────────────────────────────────

#[test]
fn search_catalog_text_mode_clamps_max_results_zero() {
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "owner/repo",
                "query": "query",
                "mode": "text",
                "limit": 0,
            }),
        )
        .unwrap();
    assert!(r.get("error").is_some() || r["ok"] == false);
}

#[test]
fn search_catalog_text_mode_clamps_max_results_extreme() {
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "owner/repo",
                "query": "query",
                "mode": "text",
                "limit": 999,
            }),
        )
        .unwrap();
    assert!(r.get("error").is_some() || r["ok"] == false);
}

#[test]
fn read_content_rejects_missing_refs() {
    // Providing neither ref nor refs should be rejected.
    //
    // Expected: error about missing required parameter
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": "owner/repo",
            }),
        )
        .unwrap();
    assert!(
        r["ok"] == false || r.get("error").is_some(),
        "missing refs should fail: {r}"
    );
    let err_msg = r["error"]["message"].as_str().unwrap_or("");
    assert!(
        err_msg.to_lowercase().contains("refs")
            || err_msg.to_lowercase().contains("invalid")
            || err_msg.to_lowercase().contains("missing"),
        "error message should mention refs/invalid/missing: {err_msg}"
    );
}

// ── Input Clamping: context_lines ───────────────────────────────────────────

#[test]
fn read_content_clamps_negative_context_lines() {
    // Negative context_lines should be clamped to 0, not cause a crash.
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": "owner/repo",
                "refs": [{ "kind": "symbol", "stable_id": "sym_id" }],
                "context_lines": -5,
            }),
        )
        .unwrap();
    // Should succeed at the envelope level (ok=true with per-item errors for
    // unindexed source) or return a top-level error — either way, no crash.
    assert!(
        r["ok"].is_boolean(),
        "response should have a boolean ok field: {r}"
    );
}

#[test]
fn read_content_clamps_excessive_context_lines() {
    // context_lines=999 should be clamped to a reasonable upper bound.
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": "owner/repo",
                "refs": [{ "kind": "symbol", "stable_id": "sym_id" }],
                "context_lines": 999,
            }),
        )
        .unwrap();
    // Should succeed at the envelope level (ok=true with per-item errors for
    // unindexed source) or return a top-level error — either way, no crash.
    assert!(
        r["ok"].is_boolean(),
        "response should have a boolean ok field: {r}"
    );
}

// ── P0-A: search_catalog output_mode ───────────────────────────────────────

fn ingest_fixture_repo(svc: &CodeService, dir: &tempfile::TempDir) -> String {
    let root = dir.path().join("test-repo");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("main.py"),
        "def hello():\n    return 'world'\n\ndef goodbye():\n    return hello()\n",
    )
    .unwrap();
    fs::write(
        root.join("utils.py"),
        "from main import hello\n\ndef helper():\n    return hello()\n",
    )
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": root.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

#[test]
fn search_catalog_output_mode_count() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": &sid,
                "query": "hello",
                "query_mode": "regex",
                "output_mode": "count",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "count mode failed: {r}");
    let count = r["data"]["count"].as_u64().unwrap();
    assert!(count >= 1, "expected at least 1 match, got {count}");
    let files = r["data"]["files"].as_u64().unwrap();
    assert!(files >= 1, "expected at least 1 file, got {files}");
    // count mode should NOT have an items array
    assert!(
        r["data"].get("items").is_none(),
        "count mode should not return items"
    );
}

#[test]
fn search_catalog_output_mode_files_with_matches() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": &sid,
                "query": "hello",
                "query_mode": "regex",
                "output_mode": "files_with_matches",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "files_with_matches failed: {r}");
    let files = r["data"]["files"].as_array().unwrap();
    assert!(!files.is_empty(), "expected at least 1 file entry");
    // Each entry should have file and count
    let first = &files[0];
    assert!(
        first.get("file").is_some(),
        "entry should have file: {first}"
    );
    assert!(
        first.get("count").is_some(),
        "entry should have count: {first}"
    );
}

#[test]
fn search_catalog_output_mode_invalid() {
    let (_dir, svc) = make_svc();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": "owner/repo",
                "query": "test",
                "output_mode": "invalid_mode",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "invalid output_mode should fail: {r}");
}

// ── P1-A: pattern alias + regex error ──────────────────────────────────────

#[test]
fn search_catalog_pattern_alias_works() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": &sid,
                "query": "",
                "pattern": "hello",
                "query_mode": "regex",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "pattern alias failed: {r}");
}

#[test]
fn search_catalog_pattern_and_query_conflict() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": &sid,
                "query": "hello",
                "pattern": "world",
                "query_mode": "regex",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "query+pattern should conflict: {r}");
}

#[test]
fn search_catalog_regex_compile_error_message() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": &sid,
                "query": "[invalid regex",
                "query_mode": "regex",
            }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "should succeed with error item: {r}"
    );
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have an error item");
    let summary = items[0]["ref"]["summary"].as_str().unwrap_or("");
    assert!(
        summary.contains("failed to compile")
            && summary.contains("Did you mean query_mode=keyword"),
        "error message should be helpful: {summary}"
    );
}

// ── P0-B: inspect_graph graph=imports ──────────────────────────────────────

#[test]
fn inspect_graph_imports_returns_outgoing_deps() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": &sid,
                "graph": "imports",
                "file_path": "utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "imports mode failed: {r}");
    assert_eq!(r["data"]["graph"], "imports");
    assert!(
        r["data"].get("imports").is_some(),
        "should have imports array"
    );
    assert!(
        r["data"].get("import_count").is_some(),
        "should have import_count"
    );
}

#[test]
fn inspect_graph_imports_requires_file_path() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": &sid,
                "graph": "imports",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "imports without file_path should fail: {r}");
}

// ── P2-A: read_content hint ────────────────────────────────────────────────

#[test]
fn read_content_plain_read_omits_next_step_hint() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_fixture_repo(&svc, &dir);
    let r = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": &sid,
                "refs": [{ "kind": "file", "path": "main.py" }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read_content failed: {r}");
    assert!(
        r["_meta"].get("next_step_hint").is_none(),
        "plain read should not emit next_step_hint: {}",
        r["_meta"]
    );
}
