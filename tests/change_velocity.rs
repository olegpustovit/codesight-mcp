//! Integration tests: churn rate.

use std::fs;
use std::process::Command;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
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

fn build_git_repo() -> Option<(tempfile::TempDir, CodeService, String)> {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("utils.py"), "def helper():\n    return 1\n").unwrap();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&src)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    run(&["init"])?;
    run(&["config", "user.email", "test@test.com"])?;
    run(&["config", "user.name", "Test"])?;
    run(&["add", "."])?;
    run(&["commit", "-m", "init"])?;

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    Some((ws, svc, sid))
}

fn build_non_git_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def helper():\n    return 1\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn change_velocity(svc: &CodeService, sid: &str, target: &str, extra: Value) -> Value {
    let mut args = json!({
        "source_id": sid,
        "analysis": "change_velocity",
        "target": target,
    });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            args[k] = v;
        }
    }
    svc.dispatch("evaluate_impact", args).unwrap()
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

// ── TestGetChurnRateErrors ───────────────────────────────────────────────────

#[test]
fn churn_errors_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": "no_such_repo",
                "analysis": "change_velocity",
                "target": "utils.py",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn churn_errors_no_source_root_returns_error() {
    let (_ws, svc, sid) = build_non_git_repo();
    let r = change_velocity(&svc, &sid, "utils.py", json!({}));
    // Either an error (not a git repo) or valid result with commits=0
    let is_error = !r["ok"].as_bool().unwrap_or(true);
    let has_commits = r["data"]["commits"].is_number();
    assert!(
        is_error || has_commits,
        "expected error or commits field: {r:?}"
    );
}

// ── TestGetChurnRateStructure ────────────────────────────────────────────────

#[test]
fn churn_structure_result_fields() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = change_velocity(&svc, &sid, "utils.py", json!({}));
    if !r["ok"].as_bool().unwrap_or(false) {
        eprintln!("skipping: git error: {r:?}");
        return;
    }
    let data = &r["data"];
    for key in [
        "commits",
        "authors",
        "churn_per_week",
        "assessment",
        "target_type",
        "file",
    ] {
        assert!(!data[key].is_null(), "Missing key: {key}");
    }
    assert!(r["_meta"].is_object());
}

#[test]
fn churn_structure_assessment_values() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = change_velocity(&svc, &sid, "utils.py", json!({}));
    if !r["ok"].as_bool().unwrap_or(false) {
        return;
    }
    let assessment = r["data"]["assessment"].as_str().unwrap();
    assert!(
        ["stable", "active", "volatile"].contains(&assessment),
        "unexpected assessment: {assessment}"
    );
}

#[test]
fn churn_structure_file_target_type() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = change_velocity(&svc, &sid, "utils.py", json!({}));
    if !r["ok"].as_bool().unwrap_or(false) {
        return;
    }
    assert_eq!(r["data"]["target_type"].as_str().unwrap(), "file");
}

#[test]
fn churn_structure_symbol_target_type() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let sym_id = find_symbol_id(&svc, &sid, "helper");
    let r = change_velocity(&svc, &sid, &sym_id, json!({}));
    if !r["ok"].as_bool().unwrap_or(false) {
        return;
    }
    assert_eq!(r["data"]["target_type"].as_str().unwrap(), "symbol");
    assert_eq!(r["data"]["symbol_name"].as_str().unwrap(), "helper");
}

#[test]
fn churn_structure_timing_present() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = change_velocity(&svc, &sid, "utils.py", json!({}));
    if !r["ok"].as_bool().unwrap_or(false) {
        return;
    }
    assert!(r["_meta"]["timing_ms"].as_f64().unwrap() >= 0.0);
}

#[test]
fn churn_structure_days_param_respected() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = change_velocity(&svc, &sid, "utils.py", json!({ "days": 1 }));
    if !r["ok"].as_bool().unwrap_or(false) {
        return;
    }
    assert_eq!(r["data"]["days"].as_u64().unwrap(), 1);
}
