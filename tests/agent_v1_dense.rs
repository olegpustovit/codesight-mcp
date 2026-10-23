//! Dense-retrieval lane — the IDF replacement surface.
//!
//! Covers the public guarantees in `IDF_REPLACEMENT.md` §4:
//!   - `semantic=true` uses the built-in model without config.
//!   - `query_mode=natural_language` implies semantic.
//!   - Pure lexical paths (`semantic=false`) do not emit semantic warnings.

use std::fs;
use std::sync::OnceLock;

use dowser_engine::config::AppConfig;
use dowser_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

/// Shared read-only fixture for dense search tests. These cases all hit the
/// same indexed Python file and only differ in search args, so re-ingesting
/// and re-embedding per test just burns ~1s of cold setup each time.
fn shared_dense_repo() -> (&'static CodeService, &'static str) {
    static SHARED: OnceLock<(TempDir, CodeService, String)> = OnceLock::new();
    let (_ws, svc, sid) = SHARED.get_or_init(build_service_with_code);
    (svc, sid.as_str())
}

fn build_service_with_code() -> (TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def hello_world():\n    return 'greetings'\n\ndef goodbye():\n    return 'bye'\n",
    )
    .unwrap();
    let svc =
        dowser_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

fn warnings(r: &Value) -> Vec<&Value> {
    r["warnings"]
        .as_array()
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}

fn has_fallback_warning(r: &Value) -> bool {
    warnings(r).iter().any(|w| w["code"] == "NL_DEGRADED")
}

#[test]
fn semantic_true_uses_built_in_model_without_fallback() {
    let (svc, sid) = shared_dense_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": sid, "query": "hello", "semantic": true, "limit": 5 }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "semantic=true must succeed with the bundled model: {r:?}"
    );
    assert!(
        !has_fallback_warning(&r),
        "bundled model should avoid fallback: {r:?}"
    );
    // Lexical items should still surface.
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "semantic path must still produce items");
}

#[test]
fn natural_language_query_uses_built_in_model_without_fallback() {
    let (svc, sid) = shared_dense_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "function that greets",
                "query_mode": "natural_language",
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "request must succeed: {r:?}");
    assert!(
        !has_fallback_warning(&r),
        "bundled model should avoid fallback: {r:?}"
    );
}

#[test]
fn lexical_only_request_does_not_emit_fallback_warning() {
    let (svc, sid) = shared_dense_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": sid, "query": "hello", "limit": 5 }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap());
    assert!(
        !has_fallback_warning(&r),
        "pure lexical query must not emit SEMANTIC_FALLBACK"
    );
}

#[test]
fn deleted_args_semantic_only_and_weight_are_rejected_silently() {
    // The fields were removed from `SearchCatalogArgs`. Our coerce-then-
    // validate stack accepts unknown keys gracefully (#[serde(default)] on
    // listed fields only). This test locks in that passing them is
    // harmless — not an error, not a validation failure.
    let (svc, sid) = shared_dense_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "semantic_only": true,
                "semantic_weight": 0.5,
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "deleted knobs must be ignored, not fail validation: {r:?}"
    );
}

#[test]
fn file_only_semantic_request_warns_but_succeeds() {
    let (svc, sid) = shared_dense_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "greetings",
                "semantic": true,
                "unit_kinds": ["file"],
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "request must succeed: {r:?}");
    let warnings = warnings(&r);
    assert!(
        warnings
            .iter()
            .any(|w| { w["code"] == "NL_DEGRADED" && w["detail"]["reason"] == "files_only" }),
        "file-only semantic path must warn with files_only fallback: {warnings:?}"
    );
}
