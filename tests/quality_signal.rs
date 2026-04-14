//! Integration tests for quality_signal, regression_check, test_gaps,
//! and snapshot_quality.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn build_multi_module_repo() -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();

    // Module A — two functions of different complexity
    let src_a = ws.path().join("src_a");
    fs::create_dir_all(&src_a).unwrap();
    fs::write(
        src_a.join("core.py"),
        r#"
from src_b.helper import do_work

def complex_func(x, y, z):
    if x > 0:
        if y > 0:
            for i in range(z):
                if i % 2 == 0:
                    while x > i:
                        x -= 1
    return x

def simple_func():
    return 42
"#,
    )
    .unwrap();

    // Module B — helper imported by A
    let src_b = ws.path().join("src_b");
    fs::create_dir_all(&src_b).unwrap();
    fs::write(
        src_b.join("helper.py"),
        r#"
def do_work():
    return "work"

def complex_func(a, b):
    if a:
        if b:
            for i in range(10):
                if i > 5:
                    return i
    return 0
"#,
    )
    .unwrap();

    // Dead file — not imported by anyone
    let orphan = ws.path().join("orphan");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("unused.py"), "def unused_thing():\n    pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (svc, ws, sid)
}

#[test]
fn quality_signal_returns_all_fields() {
    let (svc, _ws, sid) = build_multi_module_repo();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({ "source_id": sid, "analysis": "quality_signal" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "quality_signal failed: {r:?}");
    let data = &r["data"];
    assert_eq!(data["analysis"], "quality_signal");
    assert!(data["score"].is_number());
    assert!(data["structural_score"].is_number());
    assert!(data["agent_readiness_score"].is_number());
    assert!(data["assessment"].is_string());
    assert!(data["confidence"].is_string());
    assert_eq!(data["type_annotation_method"], "measured");
    assert!(data["bottleneck"].is_string());
    assert!(data["bottleneck_layer"].is_string());
    assert!(data["suggestion"].is_string());
    assert!(data["structural_root_causes"].is_object());
    assert!(data["agent_dimensions"].is_object());
    assert!(data["agent_submetrics"].is_object());
    assert!(data["coverage"].is_object());
}

#[test]
fn quality_signal_score_in_range() {
    let (svc, _ws, sid) = build_multi_module_repo();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({ "source_id": sid, "analysis": "quality_signal" }),
        )
        .unwrap();
    let score = r["data"]["score"].as_u64().unwrap();
    assert!(score <= 10000, "score {score} out of range");
}

#[test]
fn repo_health_includes_quality_signal_finding() {
    let (svc, _ws, sid) = build_multi_module_repo();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({ "source_id": sid, "analysis": "repo_health" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "repo_health failed: {r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    let qs = findings
        .iter()
        .find(|f| f["finding_id"] == "repo_health:quality_signal");
    assert!(
        qs.is_some(),
        "repo_health should include quality_signal finding"
    );
    let ev = &qs.unwrap()["evidence"];
    assert!(ev["score"].is_number(), "governance score missing");
    assert!(
        ev["structural_score"].is_number(),
        "structural score missing"
    );
    assert!(
        ev["agent_readiness_score"].is_number(),
        "agent readiness missing"
    );
    assert!(ev["structural_root_causes"].is_object());
    assert!(ev["agent_dimensions"].is_object());
    assert!(ev["agent_submetrics"].is_object());
    assert_eq!(ev["type_annotation_method"], "measured");
}

#[test]
fn snapshot_and_regression_round_trip() {
    let (svc, _ws, sid) = build_multi_module_repo();

    // Snapshot current state
    let r = svc
        .dispatch(
            "manage_indexes",
            json!({
                "source_id": sid,
                "action": "snapshot_quality",
                "snapshot_name": "test_baseline",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "snapshot_quality failed: {r:?}");
    assert!(r["data"]["stored"].as_bool().unwrap());

    // Regression check without changes → should not be degraded
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "regression_check",
                "snapshot_name": "test_baseline",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "regression_check failed: {r:?}");
    let data = &r["data"];
    assert_eq!(data["analysis"], "regression_check");
    assert!(data["baseline"].is_object());
    assert!(data["current"].is_object());
    assert!(data["delta"].is_object());
    // Score delta should be 0 (same data)
    assert_eq!(data["delta"]["score"].as_i64().unwrap(), 0);
}

#[test]
fn regression_check_without_baseline_errors() {
    let (svc, _ws, sid) = build_multi_module_repo();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "regression_check",
                "snapshot_name": "nonexistent",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap());
    assert_eq!(r["error"]["code"], "NO_BASELINE");
}

#[test]
fn test_gaps_returns_ranked_results() {
    let (svc, _ws, sid) = build_multi_module_repo();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "test_gaps",
                "top_n": 10,
                "min_complexity": 2,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "test_gaps failed: {r:?}");
    let data = &r["data"];
    assert_eq!(data["analysis"], "test_gaps");
    assert!(data["total_untested_functions"].is_number());
    assert!(data["gaps"].is_array());

    // Gaps should be sorted by risk_score descending
    let gaps = data["gaps"].as_array().unwrap();
    if gaps.len() >= 2 {
        let first = gaps[0]["risk_score"].as_f64().unwrap();
        let second = gaps[1]["risk_score"].as_f64().unwrap();
        assert!(
            first >= second,
            "gaps should be sorted descending: {first} < {second}"
        );
    }
}
