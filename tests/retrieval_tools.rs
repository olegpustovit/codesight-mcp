//! Integration tests: retrieval tools.

use std::fs;
use std::path::Path;

use chrono::{Duration, Utc};
use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_storage::db;
use serde_json::json;
use tempfile::tempdir;

fn setup_demo_repo(parent: &Path) -> std::path::PathBuf {
    let root = parent.join("retrieval_demo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("include")).unwrap();
    fs::write(
        root.join("src/main.py"),
        "def run():\n    # TODO: wire main\n    return FLAG\n",
    )
    .unwrap();
    fs::write(
        root.join("include/no_symbols.h"),
        "// TODO: wire header\n#define FLAG 1\n",
    )
    .unwrap();
    root
}

fn ingest_code(svc: &CodeService, root: &Path) -> String {
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    ing["data"]["source_id"].as_str().unwrap().to_string()
}

fn backdate_index(cfg: &AppConfig, source_id: &str, days: i64) {
    let db_path = cfg.repo_db_path(source_id);
    let old = (Utc::now() - Duration::days(days)).to_rfc3339();
    let conn = db::open_rw(&db_path).unwrap();
    conn.execute("UPDATE repo_info SET indexed_at = ?1", [&old])
        .unwrap();
}

// ── Merged: structure + catalog (read-only, shared fixture) ──

#[test]
fn case_structure_and_catalog() {
    let workspace = tempdir().unwrap();
    let root = setup_demo_repo(workspace.path());
    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);

    // file_outline: language tag present, no symbols for .h
    let fo = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "file_outline",
                "path": "include/no_symbols.h",
            }),
        )
        .unwrap();
    assert!(fo["ok"].as_bool().unwrap(), "{fo:?}");
    assert_eq!(fo["data"]["language"].as_str(), Some("cpp"));
    assert_eq!(fo["data"]["symbol_count"].as_u64(), Some(0));
    assert_eq!(fo["data"]["items"].as_array().unwrap().len(), 0);

    // repository_rollup: file counts + language breakdown
    let roll = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "repository_rollup" }),
        )
        .unwrap();
    assert!(roll["ok"].as_bool().unwrap(), "{roll:?}");
    assert_eq!(roll["data"]["file_count"], json!(2));
    let langs = roll["data"]["languages"].as_object().unwrap();
    assert_eq!(langs.get("python"), Some(&json!(1)));
    assert_eq!(langs.get("cpp"), Some(&json!(1)));
    let dirs = roll["data"]["directories"].as_object().unwrap();
    assert_eq!(dirs.get("include/"), Some(&json!(1)));
    assert_eq!(dirs.get("src/"), Some(&json!(1)));

    // catalog_sources: freshly indexed → no staleness warning
    let cat = svc.dispatch("catalog_sources", json!({})).unwrap();
    let sources = cat["data"]["sources"].as_array().unwrap();
    let entry = sources
        .iter()
        .find(|s| s["source_id"].as_str() == Some(sid.as_str()));
    assert!(entry.is_some());
    assert!(entry.unwrap().get("staleness_warning").is_none());
}

// ── Merged: search variants (read-only, shared fixture) ──

#[test]
fn case_search_variants() {
    let workspace = tempdir().unwrap();
    let root = setup_demo_repo(workspace.path());
    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);

    // context + ordering: both TODO hits with before/after lines
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "domain": "code",
                "unit_kinds": ["file"],
                "limit": 10,
                "context_lines": 1,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let items = res["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{items:?}");
    let h = items
        .iter()
        .find(|i| i["ref"]["path"].as_str() == Some("include/no_symbols.h"))
        .unwrap();
    assert_eq!(h["text"], json!("// TODO: wire header"));
    assert_eq!(h["before"].as_array().unwrap().len(), 0);
    assert_eq!(
        h["after"].as_array().unwrap(),
        &vec![json!("#define FLAG 1")]
    );
    let m = items
        .iter()
        .find(|i| i["ref"]["path"].as_str() == Some("src/main.py"))
        .unwrap();
    assert_eq!(m["before"].as_array().unwrap(), &vec![json!("def run():")]);
    assert_eq!(
        m["after"].as_array().unwrap(),
        &vec![json!("    return FLAG")]
    );

    // truncated meta: limit=1 → truncated=true
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "domain": "code",
                "unit_kinds": ["file"],
                "limit": 1,
                "context_lines": 1,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    assert_eq!(res["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        res["data"]["items"][0]["ref"]["path"].as_str(),
        Some("include/no_symbols.h")
    );
    assert_eq!(res["data"]["truncated"].as_bool(), Some(true));

    // scope: scope="src/*.py" → only Python hit
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "domain": "code",
                "unit_kinds": ["file"],
                "scope": "src/*.py",
                "limit": 10,
                "context_lines": 1,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let items = res["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["ref"]["path"].as_str(), Some("src/main.py"));

    // context clamped: context_lines=999 → same as full file
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "domain": "code",
                "unit_kinds": ["file"],
                "limit": 10,
                "context_lines": 999,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap());
    let item = res["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["ref"]["path"].as_str() == Some("src/main.py"))
        .unwrap();
    assert_eq!(
        item["before"].as_array().unwrap(),
        &vec![json!("def run():")]
    );
    assert_eq!(
        item["after"].as_array().unwrap(),
        &vec![json!("    return FLAG")]
    );
}

// ── Standalone: staleness needs DB mutation ──

#[test]
fn case_catalog_staleness_after_backdate() {
    let workspace = tempdir().unwrap();
    let root = setup_demo_repo(workspace.path());
    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg.clone()).unwrap();
    let sid = ingest_code(&svc, &root);
    backdate_index(&cfg, &sid, 8);

    let cat = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(cat["ok"].as_bool().unwrap());
    let sources = cat["data"]["sources"].as_array().unwrap();
    let entry = sources
        .iter()
        .find(|s| s["source_id"].as_str() == Some(sid.as_str()));
    assert!(entry.is_some(), "{cat:?}");
    let warn = entry.unwrap()["staleness_warning"].as_str().unwrap();
    assert!(
        warn.contains("8") && warn.contains("days"),
        "staleness_warning={warn:?}"
    );
}

// ── Standalone: removes file post-index ──

#[test]
fn case_search_skips_unreadable_file() {
    let workspace = tempdir().unwrap();
    let root = setup_demo_repo(workspace.path());
    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);
    fs::remove_file(root.join("include/no_symbols.h")).unwrap();

    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "TODO",
                "domain": "code",
                "unit_kinds": ["file"],
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    let items = res["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["ref"]["path"].as_str(), Some("src/main.py"));
}

// ── Standalone: adds extra files + removes ──

#[test]
fn case_read_file_slices_and_missing() {
    let workspace = tempdir().unwrap();
    let root = setup_demo_repo(workspace.path());
    fs::write(root.join("empty.py"), "").unwrap();
    fs::write(root.join("demo.txt"), "first\r\nsecond\r\n").unwrap();

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);

    let read = |refs| {
        svc.dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": refs,
            }),
        )
        .unwrap()
    };

    let clamp = read(json!([{
        "kind": "file",
        "path": "src/main.py",
        "start_line": 2,
        "end_line": 99
    }]));
    let d = &clamp["data"]["items"][0];
    assert_eq!(d["start_line"], json!(2));
    assert_eq!(d["end_line"], json!(3));
    assert_eq!(d["line_count"], json!(3));
    assert_eq!(d["language"], json!("python"));
    assert_eq!(
        d["content"].as_str().unwrap(),
        "    # TODO: wire main\n    return FLAG"
    );

    let rev = read(json!([{
        "kind": "file",
        "path": "src/main.py",
        "start_line": 3,
        "end_line": 1
    }]));
    let d = &rev["data"]["items"][0];
    assert_eq!(d["start_line"], json!(3));
    assert_eq!(d["end_line"], json!(3));
    assert_eq!(d["content"].as_str().unwrap(), "    return FLAG");

    let empty = read(json!([{ "kind": "file", "path": "empty.py" }]));
    let d = &empty["data"]["items"][0];
    assert_eq!(d["line_count"], json!(0));
    assert_eq!(d["start_line"], json!(0));
    assert_eq!(d["end_line"], json!(0));
    assert_eq!(d["content"].as_str().unwrap(), "");

    let verbatim = read(json!([{ "kind": "file", "path": "demo.txt" }]));
    let d = &verbatim["data"]["items"][0];
    assert_eq!(d["line_count"], json!(2));
    assert_eq!(d["start_line"], json!(1));
    assert_eq!(d["end_line"], json!(2));
    assert_eq!(d["content"].as_str().unwrap(), "first\r\nsecond\r\n");

    fs::remove_file(root.join("src/main.py")).unwrap();
    let missing = read(json!([{ "kind": "file", "path": "src/main.py" }]));
    let err = &missing["data"]["items"][0];
    assert!(err.get("error").is_some(), "{missing:?}");
    assert_eq!(err["error"]["code"].as_str(), Some("NOT_FOUND"));
}

// ── Standalone: different fixture (2 .py files, no .h) ──

#[test]
fn case_file_outline_batch_empty_and_conflict() {
    let workspace = tempdir().unwrap();
    let mini = workspace.path().join("batch_repo");
    fs::create_dir_all(mini.join("src")).unwrap();
    fs::write(mini.join("src/a.py"), "def foo(): pass\n").unwrap();
    fs::write(mini.join("src/b.py"), "def bar(): pass\n").unwrap();
    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &mini);

    let batch = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "file_outline",
                "file_paths": ["src/a.py", "src/b.py"],
            }),
        )
        .unwrap();
    assert!(batch["ok"].as_bool().unwrap(), "{batch:?}");
    let results = batch["data"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    let files: Vec<&str> = results
        .iter()
        .map(|r| r["file"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"src/a.py"));
    assert!(files.contains(&"src/b.py"));

    let empty = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "file_outline",
                "file_paths": [],
            }),
        )
        .unwrap();
    assert!(empty["ok"].as_bool().unwrap());
    assert_eq!(empty["data"]["results"].as_array().unwrap().len(), 0);

    let bad = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "file_outline",
                "path": "src/a.py",
                "file_paths": ["src/b.py"],
            }),
        )
        .unwrap();
    assert_eq!(bad["ok"], false);
}

// ── Standalone: different fixture, detects file-stat staleness ──

#[test]
fn case_sha_staleness_in_meta() {
    use std::{thread, time::Duration};

    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("staleness_check");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("app.py"), "def main():\n    pass\n").unwrap();

    let cfg = codesight_engine::config::AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            serde_json::json!({
                "path": root.to_string_lossy(), "domains": "code"
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    let sid = r["data"]["source_id"].as_str().unwrap();

    thread::sleep(Duration::from_millis(50));
    std::fs::write(root.join("app.py"), "def main():\n    print('updated')\n").unwrap();

    let health = svc
        .dispatch(
            "evaluate_quality",
            serde_json::json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    assert!(health["ok"].as_bool().unwrap(), "{health:?}");
    assert_eq!(health["data"]["analysis"], "repo_health");
}

/// `total_matches` must reflect the true pre-cap cardinality, not the clipped
/// `items.len()`. Fixture: 30 functions sharing a common name token with a
/// tiny `limit=3`.
#[test]
fn search_catalog_total_matches_is_pre_cap() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("many");
    fs::create_dir_all(&root).unwrap();
    let mut body = String::new();
    for i in 0..30 {
        body.push_str(&format!("def hotfn_{i}():\n    return {i}\n"));
    }
    fs::write(root.join("mod.py"), body).unwrap();

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);

    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hotfn",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 3,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items_len = r["data"]["items"].as_array().unwrap().len();
    let total = r["data"]["total_matches"].as_u64().unwrap() as usize;
    assert!(
        items_len <= 3,
        "items should be clipped by limit: {items_len}"
    );
    assert!(
        total >= 30,
        "total_matches must report pre-cap count (got {total}, items_len={items_len})"
    );
    assert_eq!(
        r["data"]["total_files_with_matches"].as_u64(),
        Some(1),
        "all 30 symbols live in one file"
    );
    assert_eq!(
        r["data"]["truncated"].as_bool(),
        Some(true),
        "should flag truncation when pre-cap exceeds limit"
    );
    let by: Vec<String> = r["data"]["truncated_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        by.contains(&"limit".to_string()),
        "truncated_by should include 'limit': {by:?}"
    );
}
