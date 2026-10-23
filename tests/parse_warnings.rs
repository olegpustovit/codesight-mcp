//! Integration tests: parse warnings.

use std::fs;

use serde_json::{Value, json};
use symslice_engine::config::AppConfig;
use symslice_engine::service::CodeService;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ingest(svc: &CodeService, root: &std::path::Path) -> Value {
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
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r
}

// ============================================================
// T17 -- languages_without_import_extractor
// ============================================================

#[test]
fn case_elixir_produces_languages_without_import_extractor() {
    // Indexing an Elixir file should report languages_without_import_extractor: ["elixir"].
    // Elixir has tree-sitter symbol extraction but no import extractor.
    //
    // Note: symslice-mcp does not yet have an Elixir extractor at all, so this
    // is doubly blocked: first the Elixir extractor needs to exist, then the
    // import-gap detection needs to surface languages_without_import_extractor in the response.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("app.ex"),
        "defmodule MyApp do\n  def hello(name) do\n    name\n  end\nend\n",
    )
    .unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());

    // Reference asserts: "languages_without_import_extractor" in r and "elixir" in r["languages_without_import_extractor"]
    let missing = &r["data"]["languages_without_import_extractor"];
    assert!(
        missing.is_array(),
        "Expected languages_without_import_extractor array in data"
    );
    let langs: Vec<&str> = missing
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        langs.contains(&"elixir"),
        "Expected 'elixir' in languages_without_import_extractor, got: {langs:?}"
    );
}

#[test]
fn case_languages_without_import_extractor_not_present_for_python_only() {
    // Python has both symbol and import extraction -- no languages_without_import_extractor.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("utils.py"), "def add(a, b):\n    return a + b\n").unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());

    // Python should not appear in languages_without_import_extractor (field may be absent entirely).
    let missing = &r["data"]["languages_without_import_extractor"];
    if missing.is_array() {
        let langs: Vec<&str> = missing
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            !langs.contains(&"python"),
            "python should not be in languages_without_import_extractor"
        );
    }
    // If languages_without_import_extractor is absent/null, that is fine too.
}

#[test]
fn case_languages_without_import_extractor_is_sorted_list() {
    // languages_without_import_extractor must be a sorted list (stable output).
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("app.ex"),
        "defmodule App do\n  def run(), do: :ok\nend\n",
    )
    .unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());

    let missing = &r["data"]["languages_without_import_extractor"];
    assert!(missing.is_array());
    let extractors: Vec<&str> = missing
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let mut sorted = extractors.clone();
    sorted.sort();
    assert_eq!(
        extractors, sorted,
        "languages_without_import_extractor must be sorted"
    );
}

// ============================================================
// T18 -- framework_warning in dead-code analysis
// ============================================================

fn build_no_entrypoint_repo(parent: &std::path::Path) {
    let src = parent.join("src");
    fs::create_dir_all(&src).unwrap();
    // Helper module (no entry-point filename like main.py or app.py)
    fs::write(
        src.join("helpers.py"),
        "def compute(x):\n    return x * 2\n\n\
         def transform(data):\n    return [compute(d) for d in data]\n",
    )
    .unwrap();
    fs::write(
        src.join("models.py"),
        "from helpers import compute\n\n\
         class Item:\n    def process(self, value):\n        return compute(value)\n",
    )
    .unwrap();
}

#[test]
fn case_framework_warning_present_when_no_entry_points() {
    // When no entry points exist (no main.py, app.py, etc.), framework_warning
    // must be present in get_dead_code / evaluate_quality response.
    let ws = tempdir().unwrap();
    build_no_entrypoint_repo(ws.path());
    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());
    let sid = r["data"]["source_id"].as_str().unwrap();

    let result = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    assert!(result["ok"].as_bool().unwrap());
    assert!(
        result["data"]["framework_warning"].is_string(),
        "Expected framework_warning in result"
    );
}

#[test]
fn case_framework_warning_mentions_entry_points() {
    // framework_warning text should mention "entry point".
    let ws = tempdir().unwrap();
    build_no_entrypoint_repo(ws.path());
    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());
    let sid = r["data"]["source_id"].as_str().unwrap();

    let result = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    if let Some(warning) = result["data"]["framework_warning"].as_str() {
        assert!(
            warning.to_lowercase().contains("entry point"),
            "framework_warning should mention entry points: {warning}"
        );
    } else {
        panic!("framework_warning not present");
    }
}

#[test]
fn case_no_framework_warning_when_entry_point_present() {
    // When main.py is present, framework_warning must NOT appear.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "from helpers import compute\n\nif __name__ == '__main__':\n    print(compute(5))\n",
    )
    .unwrap();
    fs::write(
        src.join("helpers.py"),
        "def compute(x):\n    return x * 2\n",
    )
    .unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());
    let sid = r["data"]["source_id"].as_str().unwrap();

    let result = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    assert!(result["ok"].as_bool().unwrap());
    assert!(
        result["data"]["framework_warning"].is_null()
            || !result["data"]
                .as_object()
                .unwrap()
                .contains_key("framework_warning"),
        "Unexpected framework_warning when main.py is present: {:?}",
        result["data"]["framework_warning"]
    );
}

#[test]
fn case_framework_warning_not_present_with_app_py() {
    // app.py is a recognised entry point -- no framework_warning.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("app.py"), "def run():\n    pass\n").unwrap();
    fs::write(src.join("utils.py"), "def helper():\n    pass\n").unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());
    let sid = r["data"]["source_id"].as_str().unwrap();

    let result = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    assert!(result["ok"].as_bool().unwrap());
    assert!(
        result["data"]["framework_warning"].is_null()
            || !result["data"]
                .as_object()
                .unwrap()
                .contains_key("framework_warning"),
        "Unexpected framework_warning when app.py is present"
    );
}

#[test]
fn case_dead_code_meta_methodology() {
    // evaluate_quality dead_code _meta should include methodology="multi_signal"
    // and confidence_level="medium".
    let ws = tempdir().unwrap();
    build_no_entrypoint_repo(ws.path());
    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = ingest(&svc, ws.path());
    let sid = r["data"]["source_id"].as_str().unwrap();

    let result = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "dead_code",
            }),
        )
        .unwrap();
    assert!(result["ok"].as_bool().unwrap());
    assert_eq!(
        result["_meta"]["methodology"].as_str(),
        Some("multi_signal"),
        "_meta.methodology should be 'multi_signal'"
    );
    assert_eq!(
        result["_meta"]["confidence_level"].as_str(),
        Some("medium"),
        "_meta.confidence_level should be 'medium'"
    );
}
