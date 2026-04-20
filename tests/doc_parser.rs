//! Integration tests: doc parser.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn doc_fixtures_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference")
}

// ── Main suite (passing tests) ──────────────────────────────────────────────

#[test]
fn parser_suite() {
    // Markdown
    run_md_basic_headings();
    run_md_heading_levels();
    run_md_fixture_sample_titles();
    run_md_slug_collision_unique_ids();
    run_md_line_offsets_valid();

    // Hierarchy
    run_hierarchy_parent_child();
    run_hierarchy_top_level_no_parent();

    // Dispatcher
    run_dispatch_md();

    // Multi-format ingest (ATX headings in all formats)
    run_multi_format_ingest();
}

// ── Ignored tests: need Rust impl work ──────────────────────────────────────

#[test]
fn doc_parser_setext_headings() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/nested/guide.md");
    let items = get_outline(&svc, &sid, "docs/nested/guide.md");
    let t = titles(&items);
    assert!(
        t.iter().any(|h| h.to_lowercase().contains("setext")),
        "expected setext heading in guide.md: {t:?}"
    );
}

#[test]
fn doc_parser_rst_underline_headings() {
    let content = "Title\n=====\n\nContent.\n\nSubsection\n----------\n\nMore.\n";
    let (ws, svc) = setup_single_doc("doc.rst", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.rst");
    let t = titles(&items);
    assert!(t.contains(&"Title".to_string()), "missing Title: {t:?}");
    assert!(
        t.contains(&"Subsection".to_string()),
        "missing Subsection: {t:?}"
    );
}

#[test]
fn doc_parser_rst_overline_heading() {
    let content = "=========\nDoc Title\n=========\n\nBody text.\n";
    let (ws, svc) = setup_single_doc("doc.rst", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.rst");
    let t = titles(&items);
    assert!(
        t.contains(&"Doc Title".to_string()),
        "missing Doc Title: {t:?}"
    );
}

#[test]
fn doc_parser_rst_level_order() {
    let content = concat!(
        "=========\nDoc Title\n=========\n\n",
        "Section\n=======\n\n",
        "Subsection\n----------\n\n",
    );
    let (ws, svc) = setup_single_doc("doc.rst", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.rst");
    let tl = titles_and_levels(&items);
    let by_title: std::collections::HashMap<&str, u64> =
        tl.iter().map(|(t, l)| (t.as_str(), *l)).collect();
    let doc_level = by_title.get("Doc Title").copied().unwrap_or(0);
    let sec_level = by_title.get("Section").copied().unwrap_or(0);
    let sub_level = by_title.get("Subsection").copied().unwrap_or(0);
    assert!(doc_level < sec_level);
    assert!(sec_level < sub_level);
}

#[test]
fn doc_parser_rst_fixture_titles() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.rst");
    let items = get_outline(&svc, &sid, "docs/sample.rst");
    let t = titles(&items);
    assert!(
        t.contains(&"Introduction".to_string()),
        "missing Introduction: {t:?}"
    );
    assert!(
        t.contains(&"Installation".to_string()),
        "missing Installation: {t:?}"
    );
    assert!(
        t.contains(&"Subsection One".to_string()),
        "missing Subsection One: {t:?}"
    );
    assert!(
        t.contains(&"Advanced Usage".to_string()),
        "missing Advanced Usage: {t:?}"
    );
}

#[test]
fn doc_parser_rst_fixture_levels() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.rst");
    let items = get_outline(&svc, &sid, "docs/sample.rst");
    let tl = titles_and_levels(&items);
    let by_title: std::collections::HashMap<&str, u64> =
        tl.iter().map(|(t, l)| (t.as_str(), *l)).collect();
    assert_eq!(by_title.get("My RST Document").copied(), Some(1));
    assert_eq!(by_title.get("Introduction").copied(), Some(2));
    assert_eq!(by_title.get("Subsection One").copied(), Some(3));
}

#[test]
fn doc_parser_rst_hierarchy_wired() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.rst");
    let items = get_outline(&svc, &sid, "docs/sample.rst");
    for item in &items {
        if item["heading"].as_str() == Some("Subsection One") {
            let hp = item["heading_path"].as_str().unwrap_or("");
            assert!(
                hp.contains("Installation"),
                "Subsection One should be under Installation: {hp:?}"
            );
        }
    }
}

#[test]
fn doc_parser_rst_unique_ids() {
    let content = "Foo\n===\n\nFirst.\n\nFoo\n===\n\nSecond.\n";
    let (ws, svc) = setup_single_doc("doc.rst", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.rst");
    let ids: Vec<&str> = items.iter().filter_map(|i| i["id"].as_str()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "RST section IDs must be unique: {ids:?}"
    );
}

#[test]
fn doc_parser_adoc_native_headings() {
    let content = "= Doc\n\nPreamble.\n\n== Section\n\nBody.\n\n=== Sub\n\nMore.\n";
    let (ws, svc) = setup_single_doc("doc.adoc", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.adoc");
    let t = titles(&items);
    assert!(t.contains(&"Doc".to_string()), "missing Doc: {t:?}");
    assert!(t.contains(&"Section".to_string()), "missing Section: {t:?}");
    assert!(t.contains(&"Sub".to_string()), "missing Sub: {t:?}");
}

#[test]
fn doc_parser_adoc_levels() {
    let content = "= H1\n\n== H2\n\n=== H3\n\n==== H4\n";
    let (ws, svc) = setup_single_doc("doc.adoc", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.adoc");
    let tl = titles_and_levels(&items);
    let by_title: std::collections::HashMap<&str, u64> =
        tl.iter().map(|(t, l)| (t.as_str(), *l)).collect();
    assert_eq!(by_title.get("H1").copied(), Some(1));
    assert_eq!(by_title.get("H2").copied(), Some(2));
    assert_eq!(by_title.get("H3").copied(), Some(3));
    assert_eq!(by_title.get("H4").copied(), Some(4));
}

#[test]
fn doc_parser_adoc_block_delimiter_not_heading() {
    let content = "== Section\n\n----\ncode block\n----\n\nBody.\n";
    let (ws, svc) = setup_single_doc("doc.adoc", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.adoc");
    let t = titles(&items);
    assert!(t.contains(&"Section".to_string()));
    assert!(
        !t.contains(&"----".to_string()),
        "block delimiters should not be headings"
    );
    assert!(
        !t.contains(&"code block".to_string()),
        "code content should not be heading"
    );
}

#[test]
fn doc_parser_adoc_fixture_titles() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.adoc");
    let items = get_outline(&svc, &sid, "docs/sample.adoc");
    let t = titles(&items);
    assert!(
        t.contains(&"My AsciiDoc Document".to_string()),
        "missing title: {t:?}"
    );
    assert!(t.contains(&"Introduction".to_string()));
    assert!(t.contains(&"Installation".to_string()));
    assert!(t.contains(&"Prerequisites".to_string()));
    assert!(t.contains(&"Advanced Usage".to_string()));
    assert!(t.contains(&"API Reference".to_string()));
}

#[test]
fn doc_parser_adoc_fixture_levels() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.adoc");
    let items = get_outline(&svc, &sid, "docs/sample.adoc");
    let tl = titles_and_levels(&items);
    let by_title: std::collections::HashMap<&str, u64> =
        tl.iter().map(|(t, l)| (t.as_str(), *l)).collect();
    assert_eq!(by_title.get("My AsciiDoc Document").copied(), Some(1));
    assert_eq!(by_title.get("Introduction").copied(), Some(2));
    assert_eq!(by_title.get("Prerequisites").copied(), Some(3));
}

#[test]
fn doc_parser_adoc_hierarchy_wired() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.adoc");
    let items = get_outline(&svc, &sid, "docs/sample.adoc");
    for item in &items {
        if item["heading"].as_str() == Some("Prerequisites") {
            let hp = item["heading_path"].as_str().unwrap_or("");
            assert!(
                hp.contains("Installation"),
                "Prerequisites under Installation: {hp:?}"
            );
        }
    }
}

#[test]
fn doc_parser_txt_paragraphs_no_headings() {
    let fixtures = doc_fixtures_root();
    let content = fs::read_to_string(fixtures.join("text/sample.txt")).unwrap();
    let ws = tempdir().unwrap();
    let text_dir = ws.path().join("text");
    fs::create_dir_all(&text_dir).unwrap();
    fs::write(text_dir.join("sample.txt"), &content).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "text/sample.txt");
    assert!(
        items.len() >= 2,
        "text file should produce ≥2 paragraph sections"
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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

fn get_outline(svc: &CodeService, sid: &str, path: &str) -> Vec<Value> {
    let r = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "document_outline",
                "path": path,
            }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "outline for {path} failed: {r:?}"
    );
    r["data"]["items"].as_array().unwrap().clone()
}

fn get_toc(svc: &CodeService, sid: &str) -> Vec<Value> {
    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "toc" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "toc failed: {r:?}");
    r["data"]["items"].as_array().unwrap().clone()
}

fn get_toc_tree(svc: &CodeService, sid: &str) -> Value {
    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "toc_tree" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "toc_tree failed: {r:?}");
    r["data"]["items"].clone()
}

fn titles(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| i["heading"].as_str().map(String::from))
        .collect()
}

fn titles_and_levels(items: &[Value]) -> Vec<(String, u64)> {
    items
        .iter()
        .filter_map(|i| {
            let t = i["heading"].as_str()?;
            let l = i["level"].as_u64()?;
            Some((t.to_string(), l))
        })
        .collect()
}

fn setup_single_doc(doc_name: &str, content: &str) -> (tempfile::TempDir, CodeService) {
    let ws = tempdir().unwrap();
    let doc_dir = ws.path().join("docs");
    fs::create_dir_all(&doc_dir).unwrap();
    fs::write(doc_dir.join(doc_name), content).unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    (ws, svc)
}

fn setup_fixture_doc(fixture_subpath: &str) -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let fixtures = doc_fixtures_root();
    let doc_dir = ws.path().join("docs");
    fs::create_dir_all(&doc_dir).unwrap();

    let fixture_path = fixtures.join(fixture_subpath);
    let fname = Path::new(fixture_subpath)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    fs::copy(&fixture_path, doc_dir.join(&fname)).unwrap();

    let nested = fixtures.join("docs/nested");
    if nested.exists() {
        let nested_dst = doc_dir.join("nested");
        fs::create_dir_all(&nested_dst).ok();
        if let Ok(entries) = fs::read_dir(&nested) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    fs::copy(entry.path(), nested_dst.join(entry.file_name())).ok();
                }
            }
        }
    }

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_docs(&svc, ws.path());
    (ws, svc, sid)
}

// ── Passing Markdown tests ──────────────────────────────────────────────────

fn run_md_basic_headings() {
    let content = "# Title\n\nIntro.\n\n## Section 1\n\nContent.\n\n## Section 2\n\nMore.\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.md");
    let t = titles(&items);
    assert!(
        t.contains(&"Section 1".to_string()),
        "missing Section 1: {t:?}"
    );
    assert!(
        t.contains(&"Section 2".to_string()),
        "missing Section 2: {t:?}"
    );
}

fn run_md_heading_levels() {
    let content = "# H1\n\n## H2\n\n### H3\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.md");
    let levels: HashSet<u64> = items.iter().filter_map(|i| i["level"].as_u64()).collect();
    assert!(levels.contains(&1), "expected level 1");
    assert!(levels.contains(&2), "expected level 2");
    assert!(levels.contains(&3), "expected level 3");
}

fn run_md_fixture_sample_titles() {
    let (_ws, svc, sid) = setup_fixture_doc("docs/sample.md");
    let items = get_outline(&svc, &sid, "docs/sample.md");
    let t = titles(&items);
    assert!(
        t.contains(&"Installation".to_string()),
        "missing Installation: {t:?}"
    );
    assert!(t.contains(&"Usage".to_string()), "missing Usage: {t:?}");
    assert!(
        t.contains(&"API Reference".to_string()),
        "missing API Reference: {t:?}"
    );
}

fn run_md_slug_collision_unique_ids() {
    let content = "## Install\n\nFirst.\n\n## Install\n\nSecond.\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.md");
    let ids: Vec<&str> = items.iter().filter_map(|i| i["id"].as_str()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "section IDs must be unique: {ids:?}"
    );
}

fn run_md_line_offsets_valid() {
    let content = "# Title\n\nContent.\n\n## Sub\n\nMore.\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.md");
    for item in &items {
        if let Some(line) = item["line"].as_u64() {
            assert!(line >= 1, "line should be 1-based");
        }
    }
}

// ── Passing hierarchy tests ─────────────────────────────────────────────────

fn run_hierarchy_parent_child() {
    let content = "# H1\n\n## H2\n\n### H3\n\n## H2b\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let tree = get_toc_tree(&svc, &sid);
    assert!(!tree.is_null(), "toc_tree should not be null");

    let items = get_outline(&svc, &sid, "docs/doc.md");
    let tl = titles_and_levels(&items);
    let h2_present = tl.iter().any(|(t, l)| t == "H2" && *l == 2);
    let h3_present = tl.iter().any(|(t, l)| t == "H3" && *l == 3);
    assert!(h2_present, "H2 at level 2 expected");
    assert!(h3_present, "H3 at level 3 expected");

    for item in &items {
        if item["heading"].as_str() == Some("H3") {
            let hp = item["heading_path"].as_str().unwrap_or("");
            assert!(
                hp.contains("H2"),
                "H3 heading_path should reference H2: {hp:?}"
            );
        }
    }
}

fn run_hierarchy_top_level_no_parent() {
    let content = "# Title\n\nContent.\n";
    let (ws, svc) = setup_single_doc("doc.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.md");
    for item in &items {
        if item["level"].as_u64() == Some(1) {
            let hp = item["heading_path"].as_str().unwrap_or("");
            assert!(
                !hp.contains(" > "),
                "top-level heading should have no parent separator: {hp:?}"
            );
        }
    }
}

// ── Passing dispatcher tests ────────────────────────────────────────────────

fn run_dispatch_md() {
    let content = "# Title\n\nContent.\n";
    let (ws, svc) = setup_single_doc("README.md", content);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/README.md");
    assert!(!items.is_empty(), "MD dispatch should produce sections");
}

fn run_multi_format_ingest() {
    let ws = tempdir().unwrap();
    let doc_dir = ws.path().join("docs");
    fs::create_dir_all(&doc_dir).unwrap();
    fs::write(doc_dir.join("readme.md"), "# Markdown\n\nBody.\n").unwrap();
    fs::write(doc_dir.join("guide.rst"), "# Guide\n\nRST with ATX.\n").unwrap();
    fs::write(doc_dir.join("manual.adoc"), "# Manual\n\nAdoc with ATX.\n").unwrap();
    fs::write(doc_dir.join("plain.txt"), "# Plain\n\nText.\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = ingest_docs(&svc, ws.path());
    let toc = get_toc(&svc, &sid);
    let t = titles(&toc);
    assert!(
        t.contains(&"Markdown".to_string()),
        "missing Markdown: {t:?}"
    );
    assert!(t.contains(&"Guide".to_string()), "missing Guide: {t:?}");
    assert!(t.contains(&"Manual".to_string()), "missing Manual: {t:?}");
    assert!(t.contains(&"Plain".to_string()), "missing Plain: {t:?}");
}
