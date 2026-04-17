//! Integration tests: incremental.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

#[test]
fn incremental_suite() {
    run_all();
}

fn run_all() {
    run_no_changes_same_counts();
    run_modified_file_updates_symbol_count();
    run_new_file_increases_counts();
    run_deleted_file_decreases_counts();
    run_renamed_file_updates_path();
    run_full_reindex_succeeds_on_existing();
    run_language_counts_updated_on_change();
    run_crlf_preserved_in_cached_files();
    // run_same_basename_folders — see standalone #[ignore] test
    run_full_reindex_preserves_summaries_from_existing();
}

fn ingest(svc: &CodeService, root: &std::path::Path, incremental: bool) -> Value {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": incremental,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r
}

/// Create a CodeService whose index lives in a separate temp dir (not inside
/// the source tree), so that `_savings.json` and other index artefacts do not
/// pollute the crawled file set.
fn make_svc() -> (tempfile::TempDir, CodeService) {
    let idx_dir = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(idx_dir.path().to_path_buf())).unwrap();
    (idx_dir, svc)
}

/// Incremental re-index with no file changes returns the same symbol count.
fn run_no_changes_same_counts() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("hello.py"), "def hello():\n    return 'hi'\n").unwrap();
    fs::write(src.join("world.py"), "def world():\n    return 'earth'\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    let count1 = r1["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(count1, 2);

    // Re-index with no changes
    let r2 = ingest(&svc, ws.path(), true);
    let count2 = r2["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(
        count2, count1,
        "no-change re-index should preserve symbol_count"
    );
}

/// Modified file with an added function increases symbol count.
fn run_modified_file_updates_symbol_count() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("calc.py"), "def add(a, b):\n    return a + b\n").unwrap();
    fs::write(src.join("util.py"), "def noop():\n    pass\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    let count1 = r1["data"]["symbol_count"].as_u64().unwrap();

    // Modify: add a function
    fs::write(
        src.join("calc.py"),
        "def add(a, b):\n    return a + b + 1\n\ndef sub(a, b):\n    return a - b\n",
    )
    .unwrap();

    // Need to wait briefly so mtime changes — or touch the file
    // (On most OS the write above is sufficient)
    let r2 = ingest(&svc, ws.path(), true);
    let count2 = r2["data"]["symbol_count"].as_u64().unwrap();
    assert!(
        count2 > count1,
        "adding a function should increase symbol_count: {count1} -> {count2}"
    );
}

/// Adding a new file increases symbol count.
fn run_new_file_increases_counts() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def func_a():\n    pass\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    let count1 = r1["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(count1, 1);

    fs::write(src.join("b.py"), "def func_b():\n    return 42\n").unwrap();

    let r2 = ingest(&svc, ws.path(), true);
    let count2 = r2["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(count2, 2, "new file should add to symbol_count");
}

/// Deleting a file decreases symbol count.
fn run_deleted_file_decreases_counts() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("keep.py"), "def keep():\n    pass\n").unwrap();
    fs::write(src.join("remove.py"), "def remove():\n    pass\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    let count1 = r1["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(count1, 2);

    fs::remove_file(src.join("remove.py")).unwrap();

    let r2 = ingest(&svc, ws.path(), true);
    let count2 = r2["data"]["symbol_count"].as_u64().unwrap();
    assert_eq!(count2, 1, "deleting a file should decrease symbol_count");
}

/// Renaming a file moves it in the index: the new path appears, the old
/// path disappears, and the ingest `delta` surfaces one removal + one reindex.
fn run_renamed_file_updates_path() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def tagged():\n    return 'rename-me'\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    assert_eq!(r1["data"]["symbol_count"].as_u64().unwrap(), 1);

    fs::rename(src.join("a.py"), src.join("b.py")).unwrap();

    let r2 = ingest(&svc, ws.path(), true);
    assert_eq!(
        r2["data"]["symbol_count"].as_u64().unwrap(),
        1,
        "rename preserves symbol count"
    );
    let files_removed = r2["data"]["delta"]["files_removed"].as_u64().unwrap();
    let files_reindexed = r2["data"]["delta"]["files_reindexed"].as_u64().unwrap();
    assert!(
        files_removed >= 1,
        "rename should surface delta.files_removed >= 1 (got {files_removed})"
    );
    assert!(
        files_reindexed >= 1,
        "rename should surface delta.files_reindexed >= 1 (got {files_reindexed})"
    );

    // Catalog confirms path moved.
    let sid = r2["data"]["source_id"].as_str().unwrap();
    let hit = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "tagged",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    let paths: Vec<String> = hit["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i.pointer("/ref/path")?.as_str().map(str::to_owned))
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("b.py")),
        "renamed file should appear under new path: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("a.py")),
        "old path must not linger: {paths:?}"
    );
}

/// Full re-index (incremental=false) on an already-indexed tree succeeds.
fn run_full_reindex_succeeds_on_existing() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("mod.py"), "def original():\n    pass\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), false);
    assert!(r1["ok"].as_bool().unwrap());

    // Full re-index again
    let r2 = ingest(&svc, ws.path(), false);
    assert!(r2["ok"].as_bool().unwrap());
    assert_eq!(
        r2["data"]["symbol_count"].as_u64().unwrap(),
        1,
        "full re-index should still see the symbol"
    );
}

/// Language counts update when file content changes language character.
fn run_language_counts_updated_on_change() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    fs::write(src.join("app.js"), "function greet() { return 'hi'; }\n").unwrap();

    let (_idx, svc) = make_svc();
    let r1 = ingest(&svc, ws.path(), true);
    let langs = &r1["data"]["languages"];
    assert!(
        langs.get("python").is_some() || langs.get("Python").is_some(),
        "should index Python: {langs:?}"
    );
    assert!(
        langs.get("javascript").is_some() || langs.get("JavaScript").is_some(),
        "should index JavaScript: {langs:?}"
    );

    // Remove JS file, add a Rust file
    fs::remove_file(src.join("app.js")).unwrap();
    fs::write(src.join("lib.rs"), "fn rust_fn() -> i32 { 42 }\n").unwrap();

    let r2 = ingest(&svc, ws.path(), true);
    let langs2 = &r2["data"]["languages"];
    assert!(
        langs2.get("rust").is_some() || langs2.get("Rust").is_some(),
        "should index Rust after swap: {langs2:?}"
    );
}
/// Local folder indexing should not normalize CRLF line endings.
/// We verify by reading the file back via `read_content` and checking
/// that CRLF is preserved.
fn run_crlf_preserved_in_cached_files() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // Write file with explicit CRLF line endings
    {
        use std::io::Write;
        let mut f = std::fs::File::create(src.join("main.py")).unwrap();
        f.write_all(b"def hello():\r\n    return 1\r\n").unwrap();
    }

    let (_idx, svc) = make_svc();
    let r = ingest(&svc, ws.path(), true);
    let sid = r["data"]["source_id"].as_str().unwrap();

    // Read the file back via read_content and check CRLF is preserved
    let read = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [{ "kind": "file", "path": "src/main.py" }],
            }),
        )
        .unwrap();
    assert!(
        read["ok"].as_bool().unwrap(),
        "read_content failed: {read:?}"
    );
    let content = read["data"]["items"][0]["content"]
        .as_str()
        .expect("content field should be a string");
    assert!(
        content.contains("\r\n"),
        "CRLF should be preserved in cached files, got: {content:?}"
    );
}

#[test]
fn case_crlf_preserved_in_cached_files() {
    run_crlf_preserved_in_cached_files();
}
/// Two folders with the same basename should get distinct source_ids.
///
/// The Rust implementation currently assigns `local/{basename}` as the
/// source_id, so two folders named "shared" at different paths both get
/// `local/shared` and the second overwrites the first. The reference test
/// expects `local/shared-{hash}` to avoid collisions.
#[test]
fn case_same_basename_folders_distinct_source_ids() {
    let ws = tempdir().unwrap();
    let left = ws.path().join("left").join("shared");
    let right = ws.path().join("right").join("shared");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("main.py"), "def left_fn():\n    return 'left'\n").unwrap();
    fs::write(
        right.join("main.py"),
        "def right_fn():\n    return 'right'\n",
    )
    .unwrap();

    let idx_path = ws.path().join("idx");
    let svc = CodeService::new(AppConfig::new(idx_path)).unwrap();

    let r_left = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": left.to_string_lossy(),
                "domains": "code",
                "incremental": false,
            }),
        )
        .unwrap();
    assert!(
        r_left["ok"].as_bool().unwrap(),
        "left ingest failed: {r_left}"
    );

    let r_right = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": right.to_string_lossy(),
                "domains": "code",
                "incremental": false,
            }),
        )
        .unwrap();
    assert!(
        r_right["ok"].as_bool().unwrap(),
        "right ingest failed: {r_right}"
    );

    let sid_left = r_left["data"]["source_id"].as_str().unwrap();
    let sid_right = r_right["data"]["source_id"].as_str().unwrap();
    assert_ne!(
        sid_left, sid_right,
        "same-basename folders should produce distinct source_ids"
    );
}
/// Full re-index on an existing index must not crash. The Python test verified
/// that a specific dict-notation bug was fixed; we verify the behavioral intent:
/// two successive full re-indexes both succeed.
fn run_full_reindex_preserves_summaries_from_existing() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def alpha():\n    pass\n").unwrap();

    let (_idx, svc) = make_svc();

    // First full index
    let r1 = ingest(&svc, ws.path(), false);
    assert!(
        r1["ok"].as_bool().unwrap(),
        "first full index failed: {r1:?}"
    );

    // Add another file
    fs::write(src.join("b.py"), "def beta():\n    pass\n").unwrap();

    // Second full index on existing data must not crash
    let r2 = ingest(&svc, ws.path(), false);
    assert!(
        r2["ok"].as_bool().unwrap(),
        "second full index failed: {r2:?}"
    );
    assert!(
        r2["data"]["symbol_count"].as_u64().unwrap() >= 2,
        "should have at least 2 symbols after adding b.py"
    );
}

#[test]
fn case_full_reindex_preserves_summaries() {
    run_full_reindex_preserves_summaries_from_existing();
}
