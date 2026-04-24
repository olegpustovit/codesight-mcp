//! Integration tests: doc broken links.

use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn ingest_docs(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn setup_broken_links_fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let doc_root = ws.path().join("docs");
    fs::create_dir_all(&doc_root).unwrap();
    for (name, content) in files {
        let path = doc_root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let sid = ingest_docs(&svc, &doc_root);
    (ws, svc, sid)
}

fn get_broken_links(svc: &CodeService, sid: &str) -> Value {
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "broken_links",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "broken_links failed: {r:?}");
    r
}

// ════════════════════════════════════════════════════════════════════════════
// Error cases
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_not_indexed() {
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": "nonexistent/repo",
                "analysis": "broken_links",
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap(),
        "should error for non-indexed repo"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Good links: zero broken
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_good_links_no_broken() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[
        (
            "index.md",
            "See [Installation](install.md) for setup.\nAlso see [Config section](install.md#configuration).",
        ),
        (
            "install.md",
            "# Installation\n\n## Configuration\n\nSet your env vars.",
        ),
    ]);
    let r = get_broken_links(&svc, &sid);
    let count = r["data"]["broken_link_count"].as_u64().unwrap();
    assert_eq!(count, 0, "no broken links expected: {r:?}");
    let links = r["data"]["broken_links"].as_array().unwrap();
    assert!(links.is_empty());
}

#[test]
fn doc_broken_links_external_links_skipped() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[(
        "index.md",
        "# Links\n\n[PyPI](https://pypi.org) and [GH](http://github.com) and [email](mailto:support@example.com)",
    )]);
    let r = get_broken_links(&svc, &sid);
    let count = r["data"]["broken_link_count"].as_u64().unwrap();
    assert_eq!(count, 0, "external links should be skipped: {r:?}");
}

#[test]
fn doc_broken_links_anchor_only_good() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[(
        "guide.md",
        "# Guide\n\nSee [Installation](#installation)\n\n## Installation\n\nSteps here.",
    )]);
    let r = get_broken_links(&svc, &sid);
    let count = r["data"]["broken_link_count"].as_u64().unwrap();
    assert_eq!(count, 0, "anchor to existing heading: {r:?}");
}

// ════════════════════════════════════════════════════════════════════════════
// Broken file link
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_broken_file_link_detected() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[(
        "index.md",
        "# Index\n\nSee [missing](missing-file.md) for details.",
    )]);
    let r = get_broken_links(&svc, &sid);
    let count = r["data"]["broken_link_count"].as_u64().unwrap();
    assert!(count >= 1, "broken file link should be detected: {r:?}");
    let links = r["data"]["broken_links"].as_array().unwrap();
    let reasons: Vec<&str> = links.iter().filter_map(|l| l["reason"].as_str()).collect();
    assert!(
        reasons.contains(&"file_not_found"),
        "reason should be file_not_found: {reasons:?}"
    );
}

#[test]
fn doc_broken_links_broken_file_link_target() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[(
        "index.md",
        "# Index\n\nSee [missing](missing-file.md) for details.",
    )]);
    let r = get_broken_links(&svc, &sid);
    let links = r["data"]["broken_links"].as_array().unwrap();
    let targets: Vec<&str> = links.iter().filter_map(|l| l["target"].as_str()).collect();
    assert!(
        targets.iter().any(|t| t.contains("missing-file.md")),
        "target should reference missing-file.md: {targets:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Broken anchor (cross-file)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_broken_cross_file_anchor() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[
        (
            "index.md",
            "# Index\n\nSee [section](install.md#no-such-section)",
        ),
        ("install.md", "# Installation\n\nContent."),
    ]);
    let r = get_broken_links(&svc, &sid);
    let count = r["data"]["broken_link_count"].as_u64().unwrap();
    assert!(
        count >= 1,
        "broken cross-file anchor should be detected: {r:?}"
    );
    let links = r["data"]["broken_links"].as_array().unwrap();
    let reasons: Vec<&str> = links.iter().filter_map(|l| l["reason"].as_str()).collect();
    assert!(
        reasons.contains(&"section_not_found"),
        "reason should be section_not_found: {reasons:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Result structure
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_result_structure() {
    let (_ws, svc, sid) =
        setup_broken_links_fixture(&[("index.md", "# Index\n\nSee [missing](missing-file.md).")]);
    let r = get_broken_links(&svc, &sid);
    let data = &r["data"];
    assert!(data["repo"].is_string(), "should have repo");
    assert!(data["docs_scanned"].is_number(), "should have docs_scanned");
    assert!(
        data["sections_scanned"].is_number(),
        "should have sections_scanned"
    );
    assert!(
        data["broken_link_count"].is_number(),
        "should have broken_link_count"
    );
    assert!(
        data["broken_links"].is_array(),
        "should have broken_links array"
    );
}

#[test]
fn doc_broken_links_entry_has_required_fields() {
    let (_ws, svc, sid) =
        setup_broken_links_fixture(&[("index.md", "# Index\n\nSee [missing](missing-file.md).")]);
    let r = get_broken_links(&svc, &sid);
    let links = r["data"]["broken_links"].as_array().unwrap();
    assert!(!links.is_empty(), "should have at least one broken link");
    let entry = &links[0];
    assert!(
        entry["source_file"].is_string(),
        "entry should have source_file"
    );
    assert!(
        entry["source_section"].is_string(),
        "entry should have source_section"
    );
    assert!(entry["target"].is_string(), "entry should have target");
    assert!(entry["reason"].is_string(), "entry should have reason");
    let reason = entry["reason"].as_str().unwrap();
    assert!(
        ["file_not_found", "section_not_found", "anchor_not_found"].contains(&reason),
        "reason should be one of the expected values: {reason}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Meta
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_broken_links_meta_timing() {
    let (_ws, svc, sid) = setup_broken_links_fixture(&[("index.md", "# Index\n\nContent.")]);
    let r = get_broken_links(&svc, &sid);
    assert!(r["_meta"].is_object(), "should have _meta");
    assert!(
        r["_meta"]["timing_ms"].is_number(),
        "should have timing_ms: {:?}",
        r["_meta"]
    );
}
