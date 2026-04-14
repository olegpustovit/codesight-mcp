//! Dense-retrieval lane — the IDF replacement surface.
//!
//! Covers the public guarantees in `IDF_REPLACEMENT.md` §4:
//!   - `semantic=true` never hard-fails when the model is absent;
//!     the request succeeds with a `SEMANTIC_FALLBACK` warning.
//!   - `query_mode=natural_language` implies semantic; same fallback rule.
//!   - Pure lexical paths (`semantic=false`) do not emit the warning.
//!
//! We deliberately point the embedder at a non-existent local path so the
//! test does not rely on network access to HuggingFace.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn force_embedder_missing() {
    // `CODESIGHT_EMBEDDING_MODEL_PATH` is consulted first and treated as a
    // local folder. A non-existent folder triggers the load-error path
    // without attempting a network fetch.
    unsafe {
        std::env::set_var(
            "CODESIGHT_EMBEDDING_MODEL_PATH",
            "/tmp/codesight-nonexistent-embedding-model",
        );
    }
}

fn build_service_with_code() -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.py"),
        "def hello_world():\n    return 'greetings'\n\ndef goodbye():\n    return 'bye'\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
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
    (svc, ws, sid)
}

fn warnings(r: &Value) -> Vec<&Value> {
    r["warnings"]
        .as_array()
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}

fn has_fallback_warning(r: &Value) -> bool {
    warnings(r)
        .iter()
        .any(|w| w["code"] == "SEMANTIC_FALLBACK")
}

#[test]
fn semantic_true_without_model_emits_fallback_warning_not_error() {
    force_embedder_missing();
    let (svc, _ws, sid) = build_service_with_code();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({ "source_id": sid, "query": "hello", "semantic": true, "limit": 5 }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "semantic=true must NOT hard-fail when model is missing: {r:?}"
    );
    assert!(
        has_fallback_warning(&r),
        "expected SEMANTIC_FALLBACK warning; got warnings={:?}",
        warnings(&r)
    );
    // Lexical items should still surface.
    let items = r["data"]["items"].as_array().unwrap();
    assert!(
        !items.is_empty(),
        "lexical fallback must still produce items"
    );
}

#[test]
fn natural_language_without_model_emits_fallback_warning() {
    force_embedder_missing();
    let (svc, _ws, sid) = build_service_with_code();
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
        has_fallback_warning(&r),
        "query_mode=natural_language must imply semantic; got warnings={:?}",
        warnings(&r)
    );
}

#[test]
fn lexical_only_request_does_not_emit_fallback_warning() {
    force_embedder_missing();
    let (svc, _ws, sid) = build_service_with_code();
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
    force_embedder_missing();
    let (svc, _ws, sid) = build_service_with_code();
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
