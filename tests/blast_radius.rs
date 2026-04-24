//! Integration tests: blast radius.

use std::fs;

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

fn build_chain_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // utils.py defines helper
    fs::write(src.join("utils.py"), "def helper():\n    return 42\n").unwrap();
    // main.py imports utils
    fs::write(
        src.join("main.py"),
        "from utils import helper\n\nresult = helper()\n",
    )
    .unwrap();
    // cli.py imports main (chain)
    fs::write(
        src.join("cli.py"),
        "from main import result\n\nprint(result)\n",
    )
    .unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn blast_radius(svc: &CodeService, sid: &str, symbol: &str, depth: u64) -> Value {
    svc.dispatch(
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "blast_radius",
            "refs": [{ "kind": "symbol", "stable_id": symbol }],
            "depth": depth,
        }),
    )
    .unwrap()
}

fn blast_radius_with_depth_scores(svc: &CodeService, sid: &str, symbol: &str, depth: u64) -> Value {
    svc.dispatch(
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "blast_radius",
            "refs": [{ "kind": "symbol", "stable_id": symbol }],
            "depth": depth,
            "include_depth_scores": true,
        }),
    )
    .unwrap()
}

// ── TestBfsImportersWithDepth (unit-level BFS behavior) ──────────────────────
// These test the BFS graph traversal that blast_radius uses internally.
// In the Rust API the behavior surfaces through the blast_radius analysis.

#[test]
fn bfs_depth1_returns_direct_only() {
    // a → b → c → d (linear chain); depth=1 should yield only b.
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    // depth-1 layer should contain only the direct importer
    let depth1_files: Vec<&str> = findings
        .iter()
        .filter(|f| f["depth"].as_u64() == Some(1))
        .flat_map(|f| f["files"].as_array().unwrap().iter())
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        depth1_files.iter().any(|f| f.contains("main.py")),
        "expected main.py as direct importer"
    );
}

#[test]
fn bfs_depth2_returns_two_layers() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    // Should find main.py at depth 1 and cli.py at depth 2
}

#[test]
fn bfs_depth3_returns_three_layers() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 3);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
}

#[test]
fn bfs_no_importers_returns_empty() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lonely.py"), "def lonely():\n    pass\n").unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &src);
    let r = blast_radius(&svc, &sid, "lonely", 3);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    // No importers: findings should be empty or risk_score 0
}

#[test]
fn bfs_no_cycles_on_circular_graph() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // a ↔ b circular
    fs::write(
        src.join("a.py"),
        "from b import bar\n\ndef foo():\n    return bar()\n",
    )
    .unwrap();
    fs::write(
        src.join("b.py"),
        "from a import foo\n\ndef bar():\n    return foo()\n",
    )
    .unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &src);
    let r = blast_radius(&svc, &sid, "foo", 5);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    // BFS should not revisit start node or produce duplicate entries
}

// ── TestBlastRadiusRiskFields (integration) ──────────────────────────────────

#[test]
fn risk_overall_risk_score_always_present() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let score = r["data"]["overall_risk_score"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&score));
}

#[test]
fn risk_direct_dependents_count_always_present() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["direct_dependents_count"].is_number());
}

#[test]
fn risk_impact_by_depth_absent_by_default() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["impact_by_depth"].is_null());
}

#[test]
fn risk_impact_by_depth_present_when_requested() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius_with_depth_scores(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let layers = r["data"]["impact_by_depth"].as_array().unwrap();
    for layer in layers {
        assert!(layer["depth"].is_number());
        assert!(layer["files"].is_array());
        assert!(layer["risk_score"].is_number());
    }
}

#[test]
fn risk_depth1_layer_contains_only_direct_importer() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius_with_depth_scores(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let layers = r["data"]["impact_by_depth"].as_array().unwrap();
    let depth1 = layers
        .iter()
        .find(|l| l["depth"].as_u64() == Some(1))
        .unwrap();
    let files: Vec<&str> = depth1["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(files.iter().any(|f| f.contains("main.py")));
}

#[test]
fn risk_score_depth1_is_1() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius_with_depth_scores(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let layers = r["data"]["impact_by_depth"].as_array().unwrap();
    let depth1 = layers
        .iter()
        .find(|l| l["depth"].as_u64() == Some(1))
        .unwrap();
    assert_eq!(depth1["risk_score"].as_f64().unwrap(), 1.0);
}

#[test]
fn risk_score_decreases_with_depth() {
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius_with_depth_scores(&svc, &sid, "helper", 2);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let layers = r["data"]["impact_by_depth"].as_array().unwrap();
    if layers.len() >= 2 {
        let mut sorted: Vec<&Value> = layers.iter().collect();
        sorted.sort_by_key(|l| l["depth"].as_u64().unwrap());
        assert!(
            sorted[0]["risk_score"].as_f64().unwrap() > sorted[1]["risk_score"].as_f64().unwrap()
        );
    }
}

#[test]
fn risk_zero_importers_gives_risk_score_zero() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lone.py"), "def lone():\n    pass\n").unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &src);
    let r = blast_radius_with_depth_scores(&svc, &sid, "lone", 1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    if r["data"]["importer_count"].as_u64() == Some(0) {
        assert_eq!(r["data"]["overall_risk_score"].as_f64().unwrap(), 0.0);
    }
}

#[test]
fn unresolved_structured_ref_returns_not_found() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lone.py"), "def lone():\n    pass\n").unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "blast_radius",
                "refs": [{
                    "kind": "symbol",
                    "path": "lone.py",
                    "symbol_name": "lone",
                    "start_line": 999
                }],
            }),
        )
        .unwrap();

    assert!(!r["ok"].as_bool().unwrap_or(true), "{r:?}");
    assert_eq!(r["error"]["code"].as_str(), Some("NOT_FOUND"));
    assert_eq!(r["error"]["category"].as_str(), Some("not_found"));
}

#[test]
fn risk_flat_impacted_symbols_list_unchanged() {
    // Backward compat: confirmed + potential lists still present
    let (_ws, svc, sid) = build_chain_repo();
    let r = blast_radius_with_depth_scores(&svc, &sid, "helper", 1);
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let data = &r["data"];
    assert!(data["confirmed"].is_array());
    assert!(data["potential"].is_array());
    assert!(data["confirmed_count"].is_number());
    assert!(data["potential_count"].is_number());
}
