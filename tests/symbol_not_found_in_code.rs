//! ROADMAP P2.1 — when `search_catalog domain=hybrid` returns only doc
//! hits for a symbol query, surface SYMBOL_NOT_FOUND_IN_CODE so the agent
//! doesn't conclude the symbol exists just because a README mentions it.

use std::fs;

use serde_json::json;
use symslice_engine::config::AppConfig;
use tempfile::tempdir;

fn build_doc_only_repo() -> (
    tempfile::TempDir,
    symslice_engine::service::CodeService,
    String,
) {
    let ws = tempdir().unwrap();
    let repo = ws.path().join("repo");
    let src = repo.join("src");
    let docs = repo.join("docs");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&docs).unwrap();
    // Code contains other symbols — not the one the agent will ask for.
    fs::write(src.join("lib.py"), "def unrelated():\n    return 1\n").unwrap();
    // The only mention of `frobnicate_widget` is in a markdown doc.
    fs::write(
        docs.join("README.md"),
        "# Design\n\nOnce upon a time `frobnicate_widget` was the entry point.\n",
    )
    .unwrap();
    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": repo.to_string_lossy(),
                "domains": "auto",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

#[test]
fn hybrid_search_warns_when_all_hits_are_doc_only() {
    let (_ws, svc, sid) = build_doc_only_repo();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "frobnicate_widget",
                "domain": "hybrid",
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let warnings = r["warnings"].as_array().cloned().unwrap_or_default();
    let has_warning = warnings
        .iter()
        .any(|w| w.get("code").and_then(|c| c.as_str()) == Some("SYMBOL_NOT_FOUND_IN_CODE"));
    assert!(
        has_warning,
        "doc-only hits must emit SYMBOL_NOT_FOUND_IN_CODE: {r:?}"
    );
}
