//! Integration tests: repo health.

use std::fs;

use dowser_engine::config::AppConfig;
use dowser_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn repo_health_suite() {
    run_all();
}

fn run_all() {
    run_returns_required_fields();
    run_summary_non_empty_string();
    run_counts_non_negative();
    run_dead_code_pct_range();
    run_top_hotspots_is_short_list();
    run_cycles_sample_is_list();
    run_timing_in_meta();
    run_missing_repo_error();
    run_repo_field_present();
    run_fn_method_count_present();
}

fn build_demo_repo(parent: &std::path::Path) -> std::path::PathBuf {
    let src = parent.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "from utils import helper\n\ndef main():\n    helper()\n",
    )
    .unwrap();
    fs::write(
        src.join("utils.py"),
        "def helper():\n    return 1\n\n\
         def dead_fn():\n    pass\n",
    )
    .unwrap();
    parent.to_path_buf()
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

fn run_returns_required_fields() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let d = &r["data"];
    for key in [
        "summary",
        "total_files",
        "total_symbols",
        "avg_complexity",
        "dead_code_pct",
        "dead_count",
        "cycle_count",
        "cycles_sample",
        "top_hotspots",
    ] {
        assert!(d.get(key).is_some(), "missing data.{key}");
    }
    assert!(r.get("_meta").is_some());
}

fn run_summary_non_empty_string() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let s = r["data"]["summary"].as_str().unwrap();
    assert!(!s.is_empty());
}

fn run_counts_non_negative() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let d = &r["data"];
    assert!(d["total_files"].as_u64().is_some());
    assert!(d["total_symbols"].as_u64().is_some());
    assert!(d["cycle_count"].as_u64().is_some());
    assert!(d["dead_count"].as_u64().is_some());
}

fn run_dead_code_pct_range() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let pct = r["data"]["dead_code_pct"].as_f64().unwrap();
    assert!((0.0..=100.0).contains(&pct));
}

fn run_top_hotspots_is_short_list() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let spots = r["data"]["top_hotspots"].as_array().unwrap();
    assert!(spots.len() <= 5);
}

fn run_cycles_sample_is_list() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    assert!(r["data"]["cycles_sample"].is_array());
}

fn run_timing_in_meta() {
    let workspace = tempdir().unwrap();
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "definitely_missing_repo_id",
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    assert!(r["_meta"]["timing_ms"].is_number());
}

fn run_missing_repo_error() {
    let workspace = tempdir().unwrap();
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false);
    assert!(r.get("error").is_some());
}

fn run_repo_field_present() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let repo = r["data"]["repo"].as_str().unwrap();
    assert!(
        repo.contains('/'),
        "expected hierarchical source_id, got {repo:?}"
    );
}

fn run_fn_method_count_present() {
    let workspace = tempdir().unwrap();
    let root = build_demo_repo(workspace.path());
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        workspace.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &root);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "repo_health",
            }),
        )
        .unwrap();
    let n = r["data"]["fn_method_count"].as_u64().unwrap();
    assert!(n >= 2, "expected main, helper, and dead_fn as functions");
}
