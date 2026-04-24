//! Integration tests: agent-v1 envelope and error paths.

use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn unknown_tool_returns_envelope_error() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().to_path_buf(),
    ))
    .unwrap();
    let r = svc.dispatch("nonexistent_tool", json!({})).unwrap();
    assert_eq!(r["ok"], false);
    assert!(r.get("error").is_some());
    assert!(r.get("_meta").is_some());
}

#[test]
fn evaluate_quality_unsupported_is_invalid_argument() {
    // Post-tightening contract: unknown `analysis` values are rejected at
    // schema validation time with INVALID_ARGUMENT instead of silently
    // falling through to a NOT_IMPLEMENTED warning.
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().to_path_buf(),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "any",
                "analysis": "some_unsupported_analysis",
            }),
        )
        .unwrap();
    assert_eq!(r["ok"], false, "{r}");
    let err = r.get("error").expect("error envelope");
    assert_eq!(err["code"], "INVALID_ARGUMENT");
}

#[test]
fn assemble_section_context_returns_primary_and_neighbors() {
    let workspace = tempdir().unwrap();
    let doc_root = workspace.path().join("docs-proj");
    fs::create_dir_all(doc_root.join("docs")).unwrap();
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference/docs/sample.md");
    fs::copy(&sample, doc_root.join("docs/sample.md")).unwrap();

    let index_root = workspace.path().join("index");
    let svc =
        codesight_mcp::test_support::make_service_from_config(AppConfig::new(index_root)).unwrap();
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": doc_root.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap());
    let sid = ing["data"]["source_id"].as_str().unwrap();

    let toc = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "toc",
            }),
        )
        .unwrap();
    assert!(toc["ok"].as_bool().unwrap());
    let items = toc["data"]["items"].as_array().unwrap();
    let target = items
        .iter()
        .find(|x| x["heading"].as_str() == Some("Prerequisites"))
        .expect("Prerequisites heading");
    let sec_id = target["id"].as_str().unwrap();

    let ctx = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": sec_id }],
                "limit": 4,
            }),
        )
        .unwrap();
    assert!(ctx["ok"].as_bool().unwrap());
    assert_eq!(ctx["data"]["mode"], "section_context");
    let prim = &ctx["data"]["section"];
    assert_eq!(prim["id"], sec_id);
    let neigh = ctx["data"]["neighbors"].as_array().unwrap();
    assert!(
        !neigh.is_empty(),
        "expected neighbor sections from same markdown file"
    );
}
