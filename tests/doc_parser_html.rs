//! Integration tests: doc parser html.

use std::fs;
use std::path::Path;

use codesight_mcp::config::AppConfig;
use codesight_retrieval::sections::{convert_html_to_markdown, extract_html_sections};
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn doc_fixtures_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference")
}

// ── Unit tests for convert_html_to_markdown ────────────────────────────────

/// Ref: TestHTMLParser.test_headings_converted
#[test]
fn html_headings_converted_to_markdown() {
    let html = "<h1>Title</h1><p>Body.</p><h2>Section</h2><p>More.</p>";
    let text = convert_html_to_markdown(html);
    assert!(text.contains("# Title"), "expected '# Title' in: {text}");
    assert!(
        text.contains("## Section"),
        "expected '## Section' in: {text}"
    );
}

/// Ref: TestHTMLParser.test_all_heading_levels
#[test]
fn html_all_heading_levels() {
    let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
    let text = convert_html_to_markdown(html);
    assert!(text.contains("# H1"), "missing # H1: {text}");
    assert!(text.contains("## H2"), "missing ## H2: {text}");
    assert!(text.contains("### H3"), "missing ### H3: {text}");
    assert!(text.contains("#### H4"), "missing #### H4: {text}");
    assert!(text.contains("##### H5"), "missing ##### H5: {text}");
    assert!(text.contains("###### H6"), "missing ###### H6: {text}");
}

/// Ref: TestHTMLParser.test_script_stripped
#[test]
fn html_script_stripped() {
    let html = "<h1>Title</h1><script>alert('xss')</script><p>Body.</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        !text.contains("alert"),
        "script content should be stripped: {text}"
    );
    assert!(
        !text.contains("xss"),
        "script content should be stripped: {text}"
    );
}

/// Ref: TestHTMLParser.test_style_stripped
#[test]
fn html_style_stripped() {
    let html = "<style>body{color:red}</style><h1>Title</h1>";
    let text = convert_html_to_markdown(html);
    assert!(
        !text.contains("color"),
        "style content should be stripped: {text}"
    );
    assert!(text.contains("# Title"), "heading should remain: {text}");
}

/// Ref: TestHTMLParser.test_nav_stripped
#[test]
fn html_nav_stripped() {
    let html = "<nav><a href='/'>Home</a></nav><h1>Title</h1>";
    let text = convert_html_to_markdown(html);
    assert!(
        !text.contains("Home"),
        "nav content should be stripped: {text}"
    );
    assert!(text.contains("# Title"), "heading should remain: {text}");
}

/// Ref: TestHTMLParser.test_footer_stripped
#[test]
fn html_footer_stripped() {
    let html = "<h1>Title</h1><footer><p>Copyright 2026</p></footer>";
    let text = convert_html_to_markdown(html);
    assert!(
        !text.contains("Copyright"),
        "footer content should be stripped: {text}"
    );
    assert!(text.contains("# Title"), "heading should remain: {text}");
}

/// Ref: TestHTMLParser.test_paragraph_text_included
#[test]
fn html_paragraph_text_included() {
    let html = "<h1>Title</h1><p>Hello world.</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("Hello world."),
        "paragraph text should be included: {text}"
    );
}

/// Ref: TestHTMLParser.test_pre_code_preserved
#[test]
fn html_pre_code_preserved() {
    let html = "<h2>Install</h2><pre><code>pip install pkg</code></pre>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("pip install pkg"),
        "code content should be preserved: {text}"
    );
    assert!(text.contains("```"), "code should be fenced: {text}");
}

/// Ref: TestHTMLParser.test_char_refs_decoded
#[test]
fn html_char_refs_decoded() {
    let html = "<h1>Caf&eacute;</h1><p>R&eacute;sum&eacute;</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("Caf\u{00e9}"),
        "named entity should be decoded: {text}"
    );
}

/// Numeric character references
#[test]
fn html_numeric_char_refs_decoded() {
    let html = "<p>&#39;hello&#39; &amp; &#x26; world</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("'hello'"),
        "numeric entity should be decoded: {text}"
    );
    assert!(text.contains("&"), "amp entity should be decoded: {text}");
}

// ── Section extraction tests (through extract_html_sections) ───────────────

/// Ref: TestHTMLParser.test_headings_converted (section-level)
#[test]
fn html_sections_from_headings() {
    let html = "<h1>Title</h1><p>Body.</p><h2>Section</h2><p>More.</p>";
    let sections = extract_html_sections("doc.html", html);
    let titles: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(titles.contains(&"Title"), "missing Title: {titles:?}");
    assert!(titles.contains(&"Section"), "missing Section: {titles:?}");
}

/// Ref: TestHTMLParser.test_fixture_titles
#[test]
fn html_fixture_titles() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let sections = extract_html_sections("sample.html", &html);
    let titles: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        titles.contains(&"Sample Documentation"),
        "missing Sample Documentation: {titles:?}"
    );
    assert!(
        titles.contains(&"Installation"),
        "missing Installation: {titles:?}"
    );
    assert!(titles.contains(&"Usage"), "missing Usage: {titles:?}");
    assert!(
        titles.contains(&"API Reference"),
        "missing API Reference: {titles:?}"
    );
}

/// Ref: TestHTMLParser.test_fixture_chrome_excluded
#[test]
fn html_fixture_chrome_excluded() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let text = convert_html_to_markdown(&html);
    assert!(
        !text.contains("console.log"),
        "script content should be stripped: {text}"
    );
    assert!(
        !text.contains("Copyright"),
        "footer content should be stripped: {text}"
    );
    assert!(
        !text.contains("Home"),
        "nav content should be stripped: {text}"
    );
}

/// Ref: TestHTMLParser.test_fixture_levels
#[test]
fn html_fixture_levels() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let sections = extract_html_sections("sample.html", &html);
    let by_title: std::collections::HashMap<&str, u8> = sections
        .iter()
        .map(|s| (s.heading.as_str(), s.level))
        .collect();
    assert_eq!(
        by_title.get("Sample Documentation").copied(),
        Some(1),
        "Sample Documentation should be level 1"
    );
    assert_eq!(
        by_title.get("Installation").copied(),
        Some(2),
        "Installation should be level 2"
    );
    assert_eq!(
        by_title.get("Prerequisites").copied(),
        Some(3),
        "Prerequisites should be level 3"
    );
}

/// Ref: TestHTMLParser.test_hierarchy_wired
#[test]
fn html_fixture_hierarchy_wired() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let sections = extract_html_sections("sample.html", &html);
    for sec in &sections {
        if sec.heading == "Prerequisites" {
            assert!(
                sec.heading_path.contains("Installation"),
                "Prerequisites should be under Installation: {:?}",
                sec.heading_path
            );
        }
    }
}

/// Ref: TestHTMLParser.test_byte_offsets_valid
/// Note: byte offsets are valid within the *converted* markdown text,
/// not the original HTML. This matches how the Rust impl stores sections.
#[test]
fn html_fixture_byte_offsets_valid() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let sections = extract_html_sections("sample.html", &html);
    assert!(!sections.is_empty(), "should have sections");
    for sec in &sections {
        assert!(
            sec.start_byte <= sec.end_byte,
            "start_byte should <= end_byte for '{}': {} > {}",
            sec.heading,
            sec.start_byte,
            sec.end_byte
        );
    }
}

// ── MCP dispatch tests (through ingest_sources + inspect_structure) ────────

/// Ref: TestHTMLParser.test_html_dispatch
#[test]
fn html_dispatch_html_extension() {
    let html = "<h1>Title</h1><p>Body.</p>";
    let (ws, svc) = setup_single_doc("doc.html", html);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/doc.html");
    let t = titles(&items);
    assert!(
        t.contains(&"Title".to_string()),
        "missing Title after .html dispatch: {t:?}"
    );
}

/// Ref: TestHTMLParser.test_htm_dispatch
#[test]
fn html_dispatch_htm_extension() {
    let html = "<h2>Section</h2><p>Content.</p>";
    let (ws, svc) = setup_single_doc("page.htm", html);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/page.htm");
    let t = titles(&items);
    assert!(
        t.contains(&"Section".to_string()),
        "missing Section after .htm dispatch: {t:?}"
    );
}

/// Full MCP fixture test: ingest sample.html and check titles via MCP
#[test]
fn html_fixture_mcp_ingest_and_outline() {
    let fixture = doc_fixtures_root().join("docs/sample.html");
    let html = fs::read_to_string(&fixture).unwrap();
    let (ws, svc) = setup_single_doc("sample.html", &html);
    let sid = ingest_docs(&svc, ws.path());
    let items = get_outline(&svc, &sid, "docs/sample.html");
    let t = titles(&items);
    assert!(
        t.contains(&"Sample Documentation".to_string()),
        "missing title: {t:?}"
    );
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

// ── Helpers ────────────────────────────────────────────────────────────────

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

fn titles(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| i["heading"].as_str().map(String::from))
        .collect()
}

fn setup_single_doc(doc_name: &str, content: &str) -> (tempfile::TempDir, CodeService) {
    let ws = tempdir().unwrap();
    let doc_dir = ws.path().join("docs");
    fs::create_dir_all(&doc_dir).unwrap();
    fs::write(doc_dir.join(doc_name), content).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    (ws, svc)
}
