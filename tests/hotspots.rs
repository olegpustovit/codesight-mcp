//! Integration tests: hotspots.

use std::fs;
use std::process::Command;

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

fn build_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("a.py"),
        concat!(
            "def simple():\n    return 1\n\n",
            "def branchy(x, y):\n",
            "    if x > 0:\n",
            "        if y > 0:\n",
            "            return x + y\n",
            "        elif y < 0:\n",
            "            return x - y\n",
            "        else:\n",
            "            return x\n",
            "    elif x < 0:\n",
            "        return -x\n",
            "    else:\n",
            "        return 0\n",
        ),
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn build_git_repo() -> Option<(tempfile::TempDir, CodeService, String)> {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("a.py"),
        concat!(
            "def simple():\n    return 1\n\n",
            "def branchy(x, y):\n",
            "    if x:\n        return x\n    return 0\n",
        ),
    )
    .unwrap();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&src)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    run(&["init"])?;
    run(&["config", "user.email", "t@t.com"])?;
    run(&["config", "user.name", "T"])?;
    run(&["add", "."])?;
    run(&["commit", "-m", "init"])?;

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    Some((ws, svc, sid))
}

fn get_hotspots(svc: &CodeService, sid: &str, extra: Value) -> Value {
    let mut args = json!({
        "source_id": sid,
        "analysis": "hotspots",
    });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            args[k] = v;
        }
    }
    svc.dispatch("evaluate_quality", args).unwrap()
}

// ── TestGetHotspots ──────────────────────────────────────────────────────────

#[test]
fn hotspots_returns_hotspots_list() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({}));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["hotspots"].is_array());
}

#[test]
fn hotspots_hotspot_fields() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    for h in r["data"]["hotspots"].as_array().unwrap() {
        assert!(h["symbol_id"].is_string(), "missing symbol_id");
        assert!(h["name"].is_string(), "missing name");
        assert!(h["file"].is_string(), "missing file");
        assert!(h["cyclomatic"].is_number(), "missing cyclomatic");
        assert!(h["churn"].is_number(), "missing churn");
        assert!(h["hotspot_score"].is_number(), "missing hotspot_score");
    }
}

#[test]
fn hotspots_top_n_respected() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({ "top_n": 1, "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["hotspots"].as_array().unwrap().len() <= 1);
}

#[test]
fn hotspots_sorted_by_score_desc() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let scores: Vec<f64> = r["data"]["hotspots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["hotspot_score"].as_f64().unwrap())
        .collect();
    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores not sorted desc: {scores:?}");
    }
}

#[test]
fn hotspots_min_complexity_filter() {
    let (_ws, svc, sid) = build_repo();
    let r_all = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    let r_high = get_hotspots(&svc, &sid, json!({ "min_complexity": 20 }));
    assert!(
        r_high["data"]["hotspots"].as_array().unwrap().len()
            <= r_all["data"]["hotspots"].as_array().unwrap().len()
    );
}

#[test]
fn hotspots_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "hotspots",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn hotspots_timing_present() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({}));
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}

#[test]
fn hotspots_git_available_field() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({}));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(!r["data"]["git_available"].is_null());
}

#[test]
fn hotspots_with_git_repo() {
    let Some((_ws, svc, sid)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    if r["data"]["git_available"].as_bool() == Some(true) {
        for h in r["data"]["hotspots"].as_array().unwrap() {
            assert!(h["churn"].as_i64().unwrap() >= 0);
        }
    }
}

#[test]
fn hotspots_assessment_field_present() {
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    for h in r["data"]["hotspots"].as_array().unwrap() {
        let assessment = h["assessment"].as_str().unwrap();
        assert!(
            ["low", "medium", "high"].contains(&assessment),
            "unexpected assessment: {assessment}"
        );
    }
}

#[test]
fn hotspots_high_complexity_no_churn_is_low() {
    // Without git history, churn=0 so hotspot_score = cyclomatic * log(1) = 0
    // All symbols should be "low" assessment
    let (_ws, svc, sid) = build_repo();
    let r = get_hotspots(&svc, &sid, json!({ "min_complexity": 1 }));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    if r["data"]["git_available"].as_bool() != Some(true) {
        for h in r["data"]["hotspots"].as_array().unwrap() {
            assert_eq!(h["assessment"].as_str().unwrap(), "low");
        }
    }
}
