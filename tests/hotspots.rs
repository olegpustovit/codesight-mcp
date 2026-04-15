//! Slice tests for hotspot scoring, plus a small MCP-envelope contract check.

use std::process::Command;

use codesight_analysis::quality::hotspots_payload;
use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::IndexStore;
use codesight_mcp::test_support::{index_code_fixture, open_code_store, write_files};
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_repo() -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[(
            "a.py",
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
        )],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

fn build_git_repo() -> Option<(tempfile::TempDir, AppConfig, String, IndexStore)> {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[(
            "a.py",
            concat!(
                "def simple():\n    return 1\n\n",
                "def branchy(x, y):\n",
                "    if x:\n        return x\n    return 0\n",
            ),
        )],
    )
    .unwrap();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&src)
            .output()
            .ok()
            .filter(|output| output.status.success())
    };
    run(&["init"])?;
    run(&["config", "user.email", "t@t.com"])?;
    run(&["config", "user.name", "T"])?;
    run(&["add", "."])?;
    run(&["commit", "-m", "init"])?;

    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).ok()?;
    let store = open_code_store(&config, &sid).ok()?;
    Some((ws, config, sid, store))
}

fn get_hotspots(store: &IndexStore, min_complexity: u64, top_n: usize) -> Value {
    hotspots_payload(store, min_complexity, top_n).unwrap()
}

#[test]
fn hotspots_returns_hotspots_list() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 0, 50);
    assert!(r["hotspots"].is_array());
}

#[test]
fn hotspots_hotspot_fields() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 1, 50);
    for hotspot in r["hotspots"].as_array().unwrap() {
        assert!(hotspot["symbol_id"].is_string(), "missing symbol_id");
        assert!(hotspot["name"].is_string(), "missing name");
        assert!(hotspot["file"].is_string(), "missing file");
        assert!(hotspot["cyclomatic"].is_number(), "missing cyclomatic");
        assert!(hotspot["churn"].is_number(), "missing churn");
        assert!(
            hotspot["hotspot_score"].is_number(),
            "missing hotspot_score"
        );
    }
}

#[test]
fn hotspots_top_n_respected() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 1, 1);
    assert!(r["hotspots"].as_array().unwrap().len() <= 1);
}

#[test]
fn hotspots_sorted_by_score_desc() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 1, 50);
    let scores: Vec<f64> = r["hotspots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["hotspot_score"].as_f64().unwrap())
        .collect();
    for pair in scores.windows(2) {
        assert!(pair[0] >= pair[1], "scores not sorted desc: {scores:?}");
    }
}

#[test]
fn hotspots_min_complexity_filter() {
    let (_ws, _config, _sid, store) = build_repo();
    let r_all = get_hotspots(&store, 1, 50);
    let r_high = get_hotspots(&store, 20, 50);
    assert!(
        r_high["hotspots"].as_array().unwrap().len() <= r_all["hotspots"].as_array().unwrap().len()
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
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(&src, &[("a.py", "def sample():\n    return 1\n")]).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "hotspots",
            }),
        )
        .unwrap();
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}

#[test]
fn hotspots_git_available_field() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 0, 50);
    assert!(!r["git_available"].is_null());
}

#[test]
fn hotspots_with_git_repo() {
    let Some((_ws, _config, _sid, store)) = build_git_repo() else {
        eprintln!("skipping: git not available");
        return;
    };
    let r = get_hotspots(&store, 1, 50);
    if r["git_available"].as_bool() == Some(true) {
        for hotspot in r["hotspots"].as_array().unwrap() {
            assert!(hotspot["churn"].as_i64().unwrap() >= 0);
        }
    }
}

#[test]
fn hotspots_assessment_field_present() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 1, 50);
    for hotspot in r["hotspots"].as_array().unwrap() {
        let assessment = hotspot["assessment"].as_str().unwrap();
        assert!(["low", "medium", "high"].contains(&assessment));
    }
}

#[test]
fn hotspots_high_complexity_no_churn_is_low() {
    let (_ws, _config, _sid, store) = build_repo();
    let r = get_hotspots(&store, 1, 50);
    if r["git_available"].as_bool() != Some(true) {
        for hotspot in r["hotspots"].as_array().unwrap() {
            assert_eq!(hotspot["assessment"].as_str().unwrap(), "low");
        }
    }
}
