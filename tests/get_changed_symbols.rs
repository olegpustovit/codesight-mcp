//! Integration tests: get changed symbols.

use std::fs;
use std::process::Command;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn git(args: &[&str], cwd: &std::path::Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("git command failed");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_head(cwd: &std::path::Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .expect("git rev-parse failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn make_git_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let root = ws.path();
    for (path, content) in files {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }
    git(&["init"], root);
    git(&["config", "user.email", "test@test.com"], root);
    git(&["config", "user.name", "Test"], root);
    git(&["add", "-A"], root);
    git(&["commit", "-m", "initial"], root);

    let svc =
        codesight_mcp::test_support::make_service_from_config(AppConfig::new(root.join("idx")))
            .unwrap();
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
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

fn commit_changes(root: &std::path::Path, changes: &[(&str, Option<&str>)]) -> String {
    for (path, content) in changes {
        let p = root.join(path);
        match content {
            Some(c) => {
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, c).unwrap();
                git(&["add", path], root);
            }
            None => {
                let _ = fs::remove_file(&p);
                git(&["rm", "--ignore-unmatch", path], root);
            }
        }
    }
    git(&["add", "-A"], root);
    git(&["commit", "-m", "changes"], root);
    git_head(root)
}

// ── Error handling ──────────────────────────────────────────────────────────

#[test]
fn test_changed_symbols_error_cases() {
    // Unknown repo
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({ "source_id": "nonexistent/repo", "analysis": "changed_units" }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");

    // GitHub repo returns not-local
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({ "source_id": "owner/testrepo", "analysis": "changed_units" }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error");
}

#[test]
fn test_changed_symbols_no_sha_at_index_time() {
    let (_ws, svc, sid) = make_git_repo(&[("x.py", "x = 1\n")]);
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({ "source_id": sid, "analysis": "changed_units" }),
        )
        .unwrap();
    assert!(r.get("_meta").is_some() || !r["ok"].as_bool().unwrap_or(true));
}

#[test]
fn test_changed_symbols_bad_since_sha() {
    let (_ws, svc, sid) = make_git_repo(&[("a.py", "def f(): pass\n")]);
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": "deadbeef00000000000000000000000000000000" }],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error for bad SHA"
    );
}

// ── Symbol detection: add / remove / modify / unchanged ─────────────────────

#[test]
fn test_changed_symbols_add_remove_modify() {
    // Add
    let (ws, svc, sid) = make_git_repo(&[("src/engine.py", "def run(): pass\n")]);
    let initial_sha = git_head(ws.path());
    commit_changes(
        ws.path(),
        &[(
            "src/engine.py",
            Some("def run(): pass\n\ndef stop(): pass\n"),
        )],
    );
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": &initial_sha }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let added: Vec<&str> = r["data"]["added_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        added.contains(&"stop"),
        "expected 'stop' in added, got {added:?}"
    );

    // Remove: re-ingest after removing stop
    let sha_before_remove = git_head(ws.path());
    commit_changes(ws.path(), &[("src/engine.py", Some("def run(): pass\n"))]);
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": &sha_before_remove }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let removed: Vec<&str> = r["data"]["removed_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        removed.contains(&"stop"),
        "expected 'stop' in removed, got {removed:?}"
    );

    // Modify
    let sha_before_mod = git_head(ws.path());
    commit_changes(
        ws.path(),
        &[("src/engine.py", Some("def run():\n    return 42\n"))],
    );
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": &sha_before_mod }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let modified: Vec<&str> = r["data"]["changed_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["change_type"].as_str() == Some("modified"))
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        modified.contains(&"run"),
        "expected 'run' in modified, got {modified:?}"
    );
}

#[test]
fn test_changed_symbols_unchanged_not_in_results() {
    let (ws, svc, sid) =
        make_git_repo(&[("src/engine.py", "def run(): pass\n\ndef stop(): pass\n")]);
    let initial_sha = git_head(ws.path());
    commit_changes(
        ws.path(),
        &[(
            "src/engine.py",
            Some("def run(): pass\n\ndef stop(): pass\n\ndef new_fn(): pass\n"),
        )],
    );
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": initial_sha }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let all_names: Vec<&str> = ["added_symbols", "removed_symbols", "changed_symbols"]
        .iter()
        .flat_map(|k| {
            r["data"][k]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|s| s["name"].as_str())
        })
        .collect();
    assert!(!all_names.contains(&"run"), "run should not appear");
    assert!(!all_names.contains(&"stop"), "stop should not appear");
    assert!(
        r["data"]["added_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"].as_str() == Some("new_fn")),
        "new_fn should be in added"
    );
}

#[test]
fn test_changed_symbols_counts_and_no_changes() {
    let (ws, svc, sid) = make_git_repo(&[("src/mod.py", "def alpha(): pass\ndef beta(): pass\n")]);
    let initial_sha = git_head(ws.path());
    commit_changes(
        ws.path(),
        &[(
            "src/mod.py",
            Some("def alpha(): return 1\ndef gamma(): pass\n"),
        )],
    );

    // Counts consistent
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": &initial_sha }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let d = &r["data"];
    assert_eq!(
        d["added_count"].as_u64().unwrap() as usize,
        d["added_symbols"].as_array().unwrap().len()
    );
    assert_eq!(
        d["removed_count"].as_u64().unwrap() as usize,
        d["removed_symbols"].as_array().unwrap().len()
    );
    assert_eq!(
        d["changed_count"].as_u64().unwrap() as usize,
        d["changed_symbols"].as_array().unwrap().len()
    );

    // No changes (same SHA twice)
    let sha = git_head(ws.path());
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [
                    { "kind": "commit", "id": &sha },
                    { "kind": "commit", "id": &sha },
                ],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(r["data"]["added_count"].as_u64().unwrap(), 0);
    assert_eq!(r["data"]["removed_count"].as_u64().unwrap(), 0);
    assert_eq!(r["data"]["changed_count"].as_u64().unwrap(), 0);

    // Suppress meta (same call, just verify no crash)
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [
                    { "kind": "commit", "id": &sha },
                    { "kind": "commit", "id": &sha },
                ],
            }),
        )
        .unwrap();
    assert!(r.get("ok").is_some());
}

#[test]
fn test_changed_symbols_include_blast_radius() {
    let (ws, svc, sid) = make_git_repo(&[
        ("src/engine.py", "def run(): pass\n"),
        ("src/main.py", "from src.engine import run\nrun()\n"),
    ]);
    let initial_sha = git_head(ws.path());
    commit_changes(
        ws.path(),
        &[("src/engine.py", Some("def run(): return 1\n"))],
    );
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "changed_units",
                "refs": [{ "kind": "commit", "id": initial_sha }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let all: Vec<&serde_json::Value> = ["changed_symbols", "added_symbols", "removed_symbols"]
        .iter()
        .flat_map(|k| r["data"][k].as_array().unwrap())
        .collect();
    for entry in all {
        assert!(
            entry.get("blast_radius").is_some(),
            "blast_radius missing on {entry:?}"
        );
    }
}
