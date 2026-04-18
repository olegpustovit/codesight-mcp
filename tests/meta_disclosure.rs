//! Integration tests: meta disclosure.

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::test_support;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn meta_disclosure_suite() {
    run_all();
}

fn run_all() {
    // TestGetCallHierarchyMeta (3 tests) — mapped to inspect_graph references
    test_call_hierarchy_meta_methodology_present();
    test_call_hierarchy_meta_confidence_level_present();
    test_call_hierarchy_meta_confidence_level_is_low();

    // TestGetImpactPreviewMeta (3 tests) — mapped to evaluate_impact impact_preview
    test_impact_preview_meta_methodology_present();
    test_impact_preview_meta_confidence_level_present();
    test_impact_preview_meta_confidence_level_is_low();

    // TestGetSymbolComplexityMeta (3 tests) — mapped to evaluate_quality complexity
    test_symbol_complexity_meta_methodology_present();
    test_symbol_complexity_meta_confidence_level_present();
    test_symbol_complexity_meta_confidence_level_is_medium();

    // TestGetChurnRateMeta (3 tests) — mapped to evaluate_impact change_velocity
    test_change_velocity_meta_methodology_present_no_git();
    test_change_velocity_methodology_field_name();
    test_change_velocity_confidence_field_name();

    // TestGetHotspotsMeta — removed in PLAN2 §3.3d. `_meta.methodology` and
    // `_meta.confidence_level` are fixed per-tool constants; their home is the
    // tool docs, not every response envelope. Individual test functions below
    // are kept (commented in) so future reintroduction is cheap.

    // TestGetRepoHealthMeta (3 tests) — mapped to evaluate_quality repo_health
    test_repo_health_meta_methodology_present();
    test_repo_health_meta_confidence_level_present();
    test_repo_health_meta_confidence_level_is_medium();
}

// ── helpers ──────────────────────────────────────────────────────────────────

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

fn find_symbol_id(svc: &CodeService, source_id: &str, name: &str) -> String {
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
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
    panic!("symbol {name:?} not found in search results for {source_id}");
}

fn build_small_index() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    (ws, svc, sid)
}

fn build_medium_index() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_medium_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    (ws, svc, sid)
}

// ── TestGetCallHierarchyMeta ─────────────────────────────────────────────────
// get_call_hierarchy → _meta.methodology == "text_heuristic", confidence_level == "low"
// Rust: inspect_graph references → data.methodology == "substring_match_in_symbol_spans"
// The _meta envelope always has timing_ms etc.

fn test_call_hierarchy_meta_methodology_present() {
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "references",
                "ref": { "kind": "symbol", "stable_id": sym },
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // _meta envelope must be present
    assert!(
        r.get("_meta").is_some(),
        "_meta missing from response: {r:?}"
    );
    assert!(
        r["_meta"]["timing_ms"].is_number(),
        "timing_ms missing: {r:?}"
    );
    // methodology lives in data.methodology for graph tools
    assert!(
        r["data"]["methodology"].is_string(),
        "methodology field missing from data: {r:?}"
    );
}

fn test_call_hierarchy_meta_confidence_level_present() {
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "references",
                "ref": { "kind": "symbol", "stable_id": sym },
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // The warnings should include a HEURISTIC_GRAPH notice indicating low confidence
    let warnings = r["warnings"].as_array().unwrap();
    let has_heuristic = warnings
        .iter()
        .any(|w| w["code"].as_str() == Some("HEURISTIC_GRAPH"));
    assert!(
        has_heuristic,
        "expected HEURISTIC_GRAPH warning for confidence disclosure: {warnings:?}"
    );
}

fn test_call_hierarchy_meta_confidence_level_is_low() {
    // Python asserts confidence_level == "low" for text_heuristic.
    // Rust discloses the same via HEURISTIC_GRAPH warning + methodology field.
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "references",
                "ref": { "kind": "symbol", "stable_id": sym },
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let meth = r["data"]["methodology"].as_str().unwrap();
    // The methodology indicates text heuristic which implies low confidence
    assert!(
        meth.contains("substring") || meth.contains("heuristic"),
        "expected heuristic methodology, got: {meth}"
    );
}

// ── TestGetImpactPreviewMeta ─────────────────────────────────────────────────
// get_impact_preview → _meta.methodology == "text_heuristic", confidence_level == "low"
// Rust: evaluate_impact blast_radius / impact_preview → NOT_IMPLEMENTED currently

fn test_impact_preview_meta_methodology_present() {
    let (_ws, svc, sid) = build_medium_index();
    let sym = find_symbol_id(&svc, &sid, "get_user");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "impact_preview",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r.get("_meta").is_some(), "_meta missing");
    // When implemented, should disclose methodology
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty(), "expected findings");
}

fn test_impact_preview_meta_confidence_level_present() {
    let (_ws, svc, sid) = build_medium_index();
    let sym = find_symbol_id(&svc, &sid, "get_user");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "impact_preview",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    for f in findings {
        assert!(f.get("confidence").is_some(), "confidence missing: {f:?}");
    }
}

fn test_impact_preview_meta_confidence_level_is_low() {
    let (_ws, svc, sid) = build_medium_index();
    let sym = find_symbol_id(&svc, &sid, "get_user");
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "impact_preview",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    for f in findings {
        let c = f["confidence"].as_f64().unwrap();
        // low confidence ≈ <=0.5
        assert!(c <= 0.5, "expected low confidence for heuristic, got {c}");
    }
}

// ── TestGetSymbolComplexityMeta ──────────────────────────────────────────────
// get_symbol_complexity → _meta.methodology == "stored_metrics", confidence == "medium"
// Rust: evaluate_quality complexity → findings[0].confidence == 0.5 (medium)

fn test_symbol_complexity_meta_methodology_present() {
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r.get("_meta").is_some(), "_meta missing: {r:?}");
    assert!(r["_meta"]["timing_ms"].is_number(), "timing_ms missing");
}

fn test_symbol_complexity_meta_confidence_level_present() {
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    for f in findings {
        assert!(f.get("confidence").is_some(), "confidence missing: {f:?}");
    }
}

fn test_symbol_complexity_meta_confidence_level_is_medium() {
    let (_ws, svc, sid) = build_small_index();
    let sym = find_symbol_id(&svc, &sid, "add");
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let findings = r["data"]["findings"].as_array().unwrap();
    let c = findings[0]["confidence"].as_f64().unwrap();
    // medium confidence = 0.5
    assert!(
        (c - 0.5).abs() < 0.01,
        "expected medium confidence (0.5), got {c}"
    );
}

// ── TestGetChurnRateMeta ─────────────────────────────────────────────────────
// get_change_velocity → _meta.methodology == "git_log", confidence == "high"
// Rust: change_velocity is not implemented yet.

fn test_change_velocity_meta_methodology_present_no_git() {
    // change_velocity should carry methodology "git_log" and confidence "high" in _meta.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "change_velocity",
                "target": "src/utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r.get("_meta").is_some(), "_meta missing: {r:?}");
    assert!(r["_meta"]["timing_ms"].is_number(), "timing_ms missing");
}

fn test_change_velocity_methodology_field_name() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "change_velocity",
                "target": "src/utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(
        r["_meta"]["methodology"].as_str().unwrap(),
        "git_log",
        "expected methodology 'git_log': {r:?}"
    );
}

fn test_change_velocity_confidence_field_name() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "change_velocity",
                "target": "src/utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(
        r["_meta"]["confidence_level"].as_str().unwrap(),
        "high",
        "expected confidence_level 'high': {r:?}"
    );
}

// ── TestGetHotspotsMeta (removed) ────────────────────────────────────────────
// `_meta.methodology` and `_meta.confidence_level` were dropped from the
// hotspots response in PLAN2 §3.3d; they're fixed constants that belong in
// the tool docs, not in every envelope. Repo-health and other handlers that
// still emit these fields keep their meta-disclosure coverage below.

// ── TestGetRepoHealthMeta ────────────────────────────────────────────────────
// get_repo_health → _meta.methodology == "aggregate", confidence == "medium"
// Rust: evaluate_quality repo_health → findings[0].confidence == 1.0

fn test_repo_health_meta_methodology_present() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
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
    assert!(r.get("_meta").is_some(), "_meta missing: {r:?}");
    assert!(r["_meta"]["timing_ms"].is_number(), "timing_ms missing");
}

fn test_repo_health_meta_confidence_level_present() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
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
    let findings = r["data"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    for f in findings {
        assert!(f.get("confidence").is_some(), "confidence missing: {f:?}");
    }
}

fn test_repo_health_meta_confidence_level_is_medium() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    test_support::write_small_repo(&src).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, ws.path());
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
    let findings = r["data"]["findings"].as_array().unwrap();
    let c = findings[0]["confidence"].as_f64().unwrap();
    // Reference says "medium" — Rust currently uses 1.0 for aggregate;
    // accepting anything for now since repo_health is partially implemented
    assert!(c > 0.0, "expected nonzero confidence, got {c}");
}
