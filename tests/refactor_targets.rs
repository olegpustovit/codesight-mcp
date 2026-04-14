//! Integration tests: extraction candidates.

use std::fs;
use std::sync::OnceLock;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

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

/// Process-lifetime shared indexed fixture. Every read-only test against
/// the same Python fixture should reuse this instead of re-ingesting. The
/// handler logic is identical across tests; only the cold ingest
/// (tree-sitter init + walk + parse + insert) dominates runtime. Tests
/// that intentionally need an empty or alternate index build their own.
fn shared_repo() -> (&'static CodeService, &'static str) {
    static SHARED: OnceLock<(TempDir, CodeService, String)> = OnceLock::new();
    let (_ws, svc, sid) = SHARED.get_or_init(build_repo);
    (svc, sid.as_str())
}

fn build_repo() -> (TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // A utility module with a complex function called from multiple files
    fs::write(
        src.join("utils.py"),
        concat!(
            "def complex_util(x, y, z):\n",
            "    if x > 0:\n",
            "        if y > 0:\n",
            "            if z > 0:\n",
            "                return x + y + z\n",
            "            else:\n",
            "                return x + y\n",
            "        elif y < 0:\n",
            "            return x - y\n",
            "        else:\n",
            "            return x\n",
            "    elif x < 0:\n",
            "        return -x\n",
            "    else:\n",
            "        return 0\n\n",
            "def simple_fn():\n",
            "    return 42\n",
        ),
    )
    .unwrap();
    fs::write(
        src.join("a.py"),
        "from utils import complex_util\nresult = complex_util(1, 2, 3)\n",
    )
    .unwrap();
    fs::write(
        src.join("b.py"),
        "from utils import complex_util\nresult = complex_util(4, 5, 6)\n",
    )
    .unwrap();
    fs::write(
        src.join("c.py"),
        "from utils import complex_util\nresult = complex_util(7, 8, 9)\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);
    (ws, svc, sid)
}

fn refactor_targets(svc: &CodeService, sid: &str, file_path: &str, extra: Value) -> Value {
    let mut args = json!({
        "source_id": sid,
        "analysis": "refactor_targets",
        "file_path": file_path,
    });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            args[k] = v;
        }
    }
    svc.dispatch("evaluate_quality", args).unwrap()
}

// ── TestGetExtractionCandidates ──────────────────────────────────────────────

#[test]
fn extraction_returns_candidates_structure() {
    let (svc, sid) = shared_repo();
    let r = refactor_targets(svc, sid, "utils.py", json!({}));
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    assert!(r["data"]["candidates"].is_array());
    assert!(r["data"]["file"].is_string());
}

#[test]
fn extraction_candidate_has_required_fields() {
    let (svc, sid) = shared_repo();
    let r = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 1, "min_callers": 1 }),
    );
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    for c in r["data"]["candidates"].as_array().unwrap() {
        assert!(c["id"].is_string(), "missing id");
        assert!(c["name"].is_string(), "missing name");
        assert!(c["cyclomatic"].is_number(), "missing cyclomatic");
        assert!(c["caller_count"].is_number(), "missing caller_count");
        assert!(c["caller_files"].is_array(), "missing caller_files");
        assert!(c["score"].is_number(), "missing score");
        assert!(c["score"].as_f64().unwrap() >= 0.0);
    }
}

#[test]
fn extraction_min_complexity_filters() {
    let (svc, sid) = shared_repo();
    let r_low = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 1, "min_callers": 1 }),
    );
    let r_high = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 100, "min_callers": 1 }),
    );
    assert!(
        r_low["data"]["candidates"].as_array().unwrap().len()
            >= r_high["data"]["candidates"].as_array().unwrap().len()
    );
}

#[test]
fn extraction_min_callers_filters() {
    let (svc, sid) = shared_repo();
    let r_low = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 1, "min_callers": 1 }),
    );
    let r_high = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 1, "min_callers": 100 }),
    );
    assert!(
        r_low["data"]["candidates"].as_array().unwrap().len()
            >= r_high["data"]["candidates"].as_array().unwrap().len()
    );
}

#[test]
fn extraction_results_sorted_by_score() {
    let (svc, sid) = shared_repo();
    let r = refactor_targets(
        svc,
        sid,
        "utils.py",
        json!({ "min_complexity": 1, "min_callers": 1 }),
    );
    assert!(r["ok"].as_bool().unwrap_or(false), "{r:?}");
    let scores: Vec<f64> = r["data"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["score"].as_f64().unwrap())
        .collect();
    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores not sorted desc: {scores:?}");
    }
}

#[test]
fn extraction_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "no_such_repo",
                "analysis": "refactor_targets",
                "file_path": "utils.py",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn extraction_missing_file_returns_error() {
    let (svc, sid) = shared_repo();
    let r = refactor_targets(svc, sid, "nonexistent_xyz.py", json!({}));
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn extraction_timing_present() {
    let (svc, sid) = shared_repo();
    let r = refactor_targets(svc, sid, "utils.py", json!({}));
    assert!(r["_meta"].is_object());
    assert!(r["_meta"]["timing_ms"].is_number());
}
