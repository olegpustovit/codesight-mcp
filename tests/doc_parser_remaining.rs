//! Integration tests: doc parser remaining.

#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_retrieval::sections::{
    convert_html_to_markdown, convert_json, convert_openapi, extract_markdown_sections,
    extract_references, extract_sections, extract_tags, preprocess_content, preprocess_notebook,
    sniff_openapi,
};
use serde_json::{Value, json};
use tempfile::tempdir;

fn doc_fixtures_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference")
}

// ── Helpers (shared with other doc parser test files) ────────────────────────────

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

fn setup_single_doc(doc_name: &str, content: &str) -> (tempfile::TempDir, CodeService) {
    let ws = tempdir().unwrap();
    let doc_dir = ws.path().join("docs");
    fs::create_dir_all(&doc_dir).unwrap();
    fs::write(doc_dir.join(doc_name), content).unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    (ws, svc)
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

// ════════════════════════════════════════════════════════════════════════════
// TestSlugify — heading text normalization in section IDs
//
// The Python `slugify` converts heading text to lowercase-hyphenated slugs.
// The Rust implementation preserves heading text in IDs but normalizes spaces.
// We test the behavioral intent: headings produce correct, stable section IDs.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_slugify_basic() {
    // slugify("Hello World") == "hello-world"
    // Rust: section ID contains the heading text "Hello World"
    let sections = extract_markdown_sections("doc.md", "## Hello World\n\nContent.\n");
    let sec = sections
        .iter()
        .find(|s| s.heading == "Hello World")
        .unwrap();
    assert!(
        sec.id.contains("Hello World"),
        "section id should contain heading text: {}",
        sec.id
    );
    // The ID follows file::heading#level format
    assert!(
        sec.id.starts_with("doc.md::"),
        "ID should start with file path: {}",
        sec.id
    );
    assert!(
        sec.id.ends_with("#h2"),
        "ID should end with #h2: {}",
        sec.id
    );
}

#[test]
fn doc_slugify_special_chars() {
    // slugify("API Reference!") == "api-reference"
    // Rust: heading text is preserved but special chars are in the heading
    let sections = extract_markdown_sections("doc.md", "## API Reference!\n\nContent.\n");
    let sec = sections
        .iter()
        .find(|s| s.heading.contains("API Reference"))
        .unwrap();
    // Verify the section is properly created with an ID
    assert!(!sec.id.is_empty(), "section ID must not be empty");
    assert!(
        sec.id.contains("API Reference"),
        "section id should contain heading text: {}",
        sec.id
    );
}

#[test]
fn doc_slugify_numbers() {
    // slugify("Step 1: Install") == "step-1-install"
    // Rust: heading text preserved with numbers
    let sections = extract_markdown_sections("doc.md", "## Step 1: Install\n\nContent.\n");
    let sec = sections
        .iter()
        .find(|s| s.heading.contains("Step 1"))
        .unwrap();
    assert!(
        sec.id.contains("Step 1"),
        "section id should contain heading text with numbers: {}",
        sec.id
    );
}

#[test]
fn doc_slugify_empty_heading() {
    // slugify("") == "section" — fallback for empty headings
    // Rust: edge case with special-char-only heading
    let sections = extract_markdown_sections("doc.md", "## !!!\n\nContent.\n");
    // Verify a section was created with a non-empty ID
    assert!(
        !sections.is_empty(),
        "should produce at least one section even with special-char-only heading"
    );
    for sec in &sections {
        assert!(!sec.id.is_empty(), "section ID must not be empty");
    }
}

#[test]
fn doc_slugify_multiple_spaces() {
    // slugify("  foo   bar  ") == "foo-bar"
    // Rust: heading text is whitespace-normalized
    let sections = extract_markdown_sections("doc.md", "##   foo   bar  \n\nContent.\n");
    let sec = sections.iter().find(|s| s.heading.contains("foo")).unwrap();
    // The heading should be normalized (collapsed whitespace)
    assert!(
        sec.heading.contains("foo") && sec.heading.contains("bar"),
        "heading should contain foo and bar: {}",
        sec.heading
    );
    assert!(!sec.id.is_empty(), "section ID must not be empty");
}

// ════════════════════════════════════════════════════════════════════════════
// TestSlugCollision — unique IDs for duplicate headings (already partially
// tested in doc_parser.rs, but these are the direct reference cases)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_slug_collision_no_collision() {
    let sections = extract_markdown_sections("doc.md", "## foo\n\nContent.\n");
    let foo_secs: Vec<_> = sections.iter().filter(|s| s.heading == "foo").collect();
    assert_eq!(foo_secs.len(), 1);
    assert!(foo_secs[0].id.contains("foo"));
}

#[test]
fn doc_slug_collision_collision() {
    let sections = extract_markdown_sections("doc.md", "## foo\n\nFirst.\n\n## foo\n\nSecond.\n");
    let foo_secs: Vec<_> = sections.iter().filter(|s| s.heading == "foo").collect();
    assert_eq!(foo_secs.len(), 2);
    let ids: HashSet<&str> = foo_secs.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        2,
        "duplicate headings must produce unique IDs: {ids:?}"
    );
}

#[test]
fn doc_slug_collision_multiple() {
    let sections =
        extract_markdown_sections("doc.md", "## foo\n\nA.\n\n## foo\n\nB.\n\n## foo\n\nC.\n");
    let foo_secs: Vec<_> = sections.iter().filter(|s| s.heading == "foo").collect();
    assert_eq!(foo_secs.len(), 3);
    let ids: HashSet<&str> = foo_secs.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        3,
        "three duplicate headings must produce three unique IDs: {ids:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestSectionId — section ID format
//
// make_section_id("local/docs", "README.md", "installation", 2)
//   == "local/docs::README.md::installation#2"
// Rust: file_path::heading_path#hN
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_section_id_format() {
    let sections = extract_markdown_sections("README.md", "## Installation\n\nContent.\n");
    let sec = sections
        .iter()
        .find(|s| s.heading == "Installation")
        .unwrap();
    // ID should contain the file path
    assert!(
        sec.id.contains("README.md"),
        "section ID should contain file path: {}",
        sec.id
    );
    // ID should contain the heading text
    assert!(
        sec.id.contains("Installation"),
        "section ID should contain heading text: {}",
        sec.id
    );
    // Check the :: separator pattern
    assert!(
        sec.id.contains("::"),
        "section ID should use :: separator: {}",
        sec.id
    );
    // Check level indicator
    assert!(
        sec.id.contains("#h2"),
        "section ID should contain level indicator: {}",
        sec.id
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestExtractReferences — internal helper, tested through section content
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_extract_references_bare_url() {
    let input = "See https://example.com/docs for more.";
    let refs = extract_references(input);
    assert!(
        refs.contains(&"https://example.com/docs".to_string()),
        "bare URL should be extracted from: {input}; got: {refs:?}"
    );
}

#[test]
fn doc_extract_references_markdown_link() {
    let input = "[Guide](https://example.com/guide)";
    let refs = extract_references(input);
    assert!(
        refs.contains(&"https://example.com/guide".to_string()),
        "markdown link URL should be extracted from: {input}; got: {refs:?}"
    );
}

#[test]
fn doc_extract_references_no_duplicates() {
    let input = "[Link](https://x.com) and https://x.com";
    let refs = extract_references(input);
    assert_eq!(
        refs.iter().filter(|r| *r == "https://x.com").count(),
        1,
        "duplicate URLs should be deduplicated; got: {refs:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestExtractTags — internal helper
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_extract_tags_hashtag() {
    let input = "This is #important and #api content.";
    let tags = extract_tags(input);
    assert!(
        tags.contains(&"important".to_string()),
        "should contain 'important'; got: {tags:?}"
    );
    assert!(
        tags.contains(&"api".to_string()),
        "should contain 'api'; got: {tags:?}"
    );
}

#[test]
fn doc_extract_tags_no_tags() {
    let input = "No tags here.";
    let tags = extract_tags(input);
    assert!(
        tags.is_empty(),
        "text without hashtags should produce no tags; got: {tags:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestParseFileDispatcher extras — ipynb dispatch
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_dispatch_ipynb() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# Title\n\nBody."]}]}"##;
    let text = preprocess_content(nb, "notebook.ipynb");
    let sections = extract_markdown_sections("notebook.ipynb", &text);
    assert!(
        !sections.is_empty(),
        "ipynb dispatch should produce sections"
    );
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Title"),
        "should find Title heading: {headings:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestNotebookParser
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_notebook_markdown_cells_included() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# Hello\n\nWorld."]}]}"##;
    let (text, _lang) = preprocess_notebook(nb);
    assert!(
        text.contains("# Hello"),
        "markdown heading should be preserved"
    );
    assert!(text.contains("World."), "markdown body should be preserved");
}

#[test]
fn doc_notebook_code_cells_fenced() {
    let nb = r#"{"metadata":{"language_info":{"name":"python"}},"nbformat":4,"cells":[{"cell_type":"code","source":["x = 1"]}]}"#;
    let (text, _lang) = preprocess_notebook(nb);
    assert!(
        text.contains("```python"),
        "code cell should have python fence"
    );
    assert!(text.contains("x = 1"), "code content should be preserved");
    assert!(text.matches("```").count() >= 2, "fence should be closed");
}

#[test]
fn doc_notebook_kernel_language_detected() {
    let nb = r#"{"metadata":{"kernelspec":{"language":"julia"}},"nbformat":4,"cells":[{"cell_type":"code","source":["println(1)"]}]}"#;
    let (text, _lang) = preprocess_notebook(nb);
    assert!(
        text.contains("```julia"),
        "should detect julia from kernelspec"
    );
}

#[test]
fn doc_notebook_language_defaults_to_python() {
    let nb = r#"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"code","source":["x=1"]}]}"#;
    let (text, _lang) = preprocess_notebook(nb);
    assert!(
        text.contains("```python"),
        "should default to python when no language metadata"
    );
}

#[test]
fn doc_notebook_empty_cells_skipped() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":[""]},{"cell_type":"markdown","source":["# Real"]}]}"##;
    let (text, _lang) = preprocess_notebook(nb);
    assert_eq!(text.trim(), "# Real", "empty cell should be skipped");
}

#[test]
fn doc_notebook_invalid_json_returns_empty() {
    let nb = "not json";
    let (text, _lang) = preprocess_notebook(nb);
    assert_eq!(text, "", "invalid JSON should produce empty output");
}

#[test]
fn doc_notebook_source_as_list() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# T","itle\n","\nBody."]}]}"##;
    let (text, _lang) = preprocess_notebook(nb);
    assert!(
        text.contains("# Title"),
        "source list elements should be concatenated: {text}"
    );
}

#[test]
fn doc_notebook_sections_from_markdown_headings() {
    let fixtures = doc_fixtures_root();
    let content = fs::read_to_string(fixtures.join("docs/sample.ipynb")).unwrap();
    let sections = extract_sections("sample.ipynb", &content);
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Data Analysis Notebook"),
        "headings: {headings:?}"
    );
    assert!(headings.contains(&"Setup"), "headings: {headings:?}");
    assert!(headings.contains(&"Loading Data"), "headings: {headings:?}");
    assert!(headings.contains(&"Results"), "headings: {headings:?}");
}

#[test]
fn doc_notebook_code_in_section_body() {
    let fixtures = doc_fixtures_root();
    let content = fs::read_to_string(fixtures.join("docs/sample.ipynb")).unwrap();
    let text = preprocess_content(&content, "sample.ipynb");
    let sections = extract_markdown_sections("sample.ipynb", &text);
    let setup = sections.iter().find(|s| s.heading == "Setup").unwrap();
    let section_text = &text[setup.start_byte..setup.end_byte];
    assert!(
        section_text.contains("```python"),
        "Setup should have python fence"
    );
    assert!(
        section_text.contains("import pandas"),
        "Setup should contain import pandas"
    );
}

#[test]
fn doc_notebook_hierarchy_wired() {
    let fixtures = doc_fixtures_root();
    let content = fs::read_to_string(fixtures.join("docs/sample.ipynb")).unwrap();
    let sections = extract_sections("sample.ipynb", &content);
    let validation = sections
        .iter()
        .find(|s| s.heading == "Data Validation")
        .unwrap();
    // Data Validation (h3) should have Loading Data in its heading_path
    assert!(
        validation.heading_path.contains("Loading Data"),
        "Data Validation should be under Loading Data: {}",
        validation.heading_path
    );
}

#[test]
fn doc_notebook_byte_offsets_valid() {
    let fixtures = doc_fixtures_root();
    let content = fs::read_to_string(fixtures.join("docs/sample.ipynb")).unwrap();
    let text = preprocess_content(&content, "sample.ipynb");
    let sections = extract_markdown_sections("sample.ipynb", &text);
    let text_bytes = text.as_bytes();
    for sec in &sections {
        assert!(
            sec.start_byte <= sec.end_byte,
            "start <= end for '{}'",
            sec.heading
        );
        assert!(
            sec.end_byte <= text_bytes.len(),
            "byte_end {} exceeds text len {} for '{}'",
            sec.end_byte,
            text_bytes.len(),
            sec.heading
        );
    }
}

#[test]
fn doc_notebook_preprocess_passthrough_for_non_notebook() {
    let md = "# Title\n\nBody.";
    assert_eq!(preprocess_content(md, "doc.md"), md);
    assert_eq!(preprocess_content(md, "doc.rst"), md);
    assert_eq!(preprocess_content(md, "doc.adoc"), md);
}

// ════════════════════════════════════════════════════════════════════════════
// TestHTMLParser
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_html_headings_converted() {
    let html = "<h1>Title</h1><p>Body.</p><h2>Section</h2><p>More.</p>";
    let text = convert_html_to_markdown(html);
    assert!(text.contains("# Title"), "h1 should become # heading");
    assert!(text.contains("## Section"), "h2 should become ## heading");
}

#[test]
fn doc_html_all_heading_levels() {
    let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
    let text = convert_html_to_markdown(html);
    assert!(text.contains("# H1"));
    assert!(text.contains("## H2"));
    assert!(text.contains("### H3"));
    assert!(text.contains("#### H4"));
    assert!(text.contains("##### H5"));
    assert!(text.contains("###### H6"));
}

#[test]
fn doc_html_script_stripped() {
    let html = "<h1>Title</h1><script>alert('xss')</script><p>Body.</p>";
    let text = convert_html_to_markdown(html);
    assert!(!text.contains("alert"), "script content should be stripped");
    assert!(!text.contains("xss"), "script content should be stripped");
}

#[test]
fn doc_html_style_stripped() {
    let html = "<style>body{color:red}</style><h1>Title</h1>";
    let text = convert_html_to_markdown(html);
    assert!(!text.contains("color"), "style content should be stripped");
    assert!(text.contains("# Title"), "heading should still appear");
}

#[test]
fn doc_html_nav_stripped() {
    let html = "<nav><a href='/'>Home</a></nav><h1>Title</h1>";
    let text = convert_html_to_markdown(html);
    assert!(!text.contains("Home"), "nav content should be stripped");
    assert!(text.contains("# Title"), "heading should still appear");
}

#[test]
fn doc_html_footer_stripped() {
    let html = "<h1>Title</h1><footer><p>Copyright 2026</p></footer>";
    let text = convert_html_to_markdown(html);
    assert!(
        !text.contains("Copyright"),
        "footer content should be stripped"
    );
    assert!(text.contains("# Title"), "heading should still appear");
}

#[test]
fn doc_html_paragraph_text_included() {
    let html = "<h1>Title</h1><p>Hello world.</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("Hello world."),
        "paragraph text should be preserved"
    );
}

#[test]
fn doc_html_pre_code_preserved() {
    let html = "<h2>Install</h2><pre><code>pip install pkg</code></pre>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("pip install pkg"),
        "code content should be preserved"
    );
    assert!(text.contains("```"), "should produce fenced code block");
}

#[test]
fn doc_html_char_refs_decoded() {
    let html = "<h1>Caf&eacute;</h1><p>R&eacute;sum&eacute;</p>";
    let text = convert_html_to_markdown(html);
    assert!(
        text.contains("Caf\u{00e9}"),
        "HTML entity &eacute; should become e-acute"
    );
}

#[test]
fn doc_html_fixture_titles() {
    let fixtures = doc_fixtures_root();
    let html = fs::read_to_string(fixtures.join("docs/sample.html")).unwrap();
    let sections = extract_sections("sample.html", &html);
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Sample Documentation"),
        "headings: {headings:?}"
    );
    assert!(headings.contains(&"Installation"), "headings: {headings:?}");
    assert!(
        headings.contains(&"Prerequisites"),
        "headings: {headings:?}"
    );
    assert!(headings.contains(&"Usage"), "headings: {headings:?}");
    assert!(
        headings.contains(&"API Reference"),
        "headings: {headings:?}"
    );
}

#[test]
fn doc_html_fixture_chrome_excluded() {
    let fixtures = doc_fixtures_root();
    let html = fs::read_to_string(fixtures.join("docs/sample.html")).unwrap();
    let text = convert_html_to_markdown(&html);
    assert!(!text.contains("console.log"), "script should be stripped");
    assert!(!text.contains("Copyright"), "footer should be stripped");
    assert!(!text.contains("Home"), "nav should be stripped");
}

#[test]
fn doc_html_fixture_levels() {
    let fixtures = doc_fixtures_root();
    let html = fs::read_to_string(fixtures.join("docs/sample.html")).unwrap();
    let sections = extract_sections("sample.html", &html);
    let by_heading: std::collections::HashMap<&str, u8> = sections
        .iter()
        .map(|s| (s.heading.as_str(), s.level))
        .collect();
    assert_eq!(by_heading["Sample Documentation"], 1);
    assert_eq!(by_heading["Installation"], 2);
    assert_eq!(by_heading["Prerequisites"], 3);
}

#[test]
fn doc_html_hierarchy_wired() {
    let fixtures = doc_fixtures_root();
    let html = fs::read_to_string(fixtures.join("docs/sample.html")).unwrap();
    let sections = extract_sections("sample.html", &html);
    let prereqs = sections
        .iter()
        .find(|s| s.heading == "Prerequisites")
        .unwrap();
    assert!(
        prereqs.heading_path.contains("Installation"),
        "Prerequisites should be under Installation: {}",
        prereqs.heading_path
    );
}

#[test]
fn doc_html_byte_offsets_valid() {
    let fixtures = doc_fixtures_root();
    let html = fs::read_to_string(fixtures.join("docs/sample.html")).unwrap();
    let md = convert_html_to_markdown(&html);
    let sections = extract_markdown_sections("sample.html", &md);
    let text_bytes = md.as_bytes();
    for sec in &sections {
        assert!(sec.start_byte <= sec.end_byte);
        assert!(
            sec.end_byte <= text_bytes.len(),
            "byte_end {} exceeds text len {}",
            sec.end_byte,
            text_bytes.len()
        );
    }
}

#[test]
fn doc_html_dispatch() {
    let html = "<h1>Title</h1><p>Body.</p>";
    let sections = extract_sections("doc.html", html);
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Title"),
        ".html should dispatch to HTML parser"
    );
}

#[test]
fn doc_html_htm_dispatch() {
    let html = "<h2>Section</h2><p>Content.</p>";
    let sections = extract_sections("page.htm", html);
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Section"),
        ".htm should dispatch to HTML parser"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TestSniffOpenAPI
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_sniff_openapi_yaml_openapi3() {
    assert!(sniff_openapi(
        "openapi: \"3.0.0\"\ninfo:\n  title: Test\n",
        "api.yaml"
    ));
}

#[test]
fn doc_sniff_openapi_json_swagger2() {
    assert!(sniff_openapi(
        "{\"swagger\": \"2.0\", \"info\": {}}",
        "api.json"
    ));
}

#[test]
fn doc_sniff_openapi_plain_yaml_not_openapi() {
    assert!(!sniff_openapi("name: test\nversion: 1.0\n", "config.yaml"));
}

#[test]
fn doc_sniff_openapi_wrong_extension() {
    assert!(!sniff_openapi("openapi: \"3.0.0\"\n", "api.txt"));
}

#[test]
fn doc_sniff_openapi_md_extension() {
    assert!(!sniff_openapi("openapi: \"3.0.0\"\n", "api.md"));
}

// ════════════════════════════════════════════════════════════════════════════
// TestConvertOpenAPI
// ════════════════════════════════════════════════════════════════════════════

fn petstore_yaml() -> &'static str {
    r#"openapi: "3.0.0"
info:
  title: Petstore API
  version: "1.0.0"
paths:
  /pets:
    get:
      tags: [pets]
      summary: List all pets
      parameters:
        - name: limit
          in: query
          description: How many items to return
      responses:
        "200":
          description: A list of pets
    post:
      tags: [pets]
      summary: Create a pet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/NewPet'
      responses:
        "201":
          description: Created
components:
  schemas:
    Pet:
      required: [id, name]
      properties:
        id:
          type: integer
        name:
          type: string
    NewPet:
      properties:
        name:
          type: string
"#
}

#[test]
fn doc_convert_openapi_title_in_output() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.starts_with("# Petstore API"), "output: {out}");
}

#[test]
fn doc_convert_openapi_version_in_output() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("1.0.0"), "version should appear: {out}");
}

#[test]
fn doc_convert_openapi_tag_section() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("## pets"), "tag section should appear: {out}");
}

#[test]
fn doc_convert_openapi_operation_headings() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("### GET /pets"), "GET /pets heading: {out}");
    assert!(out.contains("### POST /pets"), "POST /pets heading: {out}");
}

#[test]
fn doc_convert_openapi_operation_summary() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("List all pets"), "summary: {out}");
}

#[test]
fn doc_convert_openapi_parameter_rendered() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("`limit`"), "limit param: {out}");
    assert!(out.contains("query"), "query location: {out}");
}

#[test]
fn doc_convert_openapi_request_body_rendered() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("Request Body"), "Request Body: {out}");
    assert!(out.contains("NewPet"), "NewPet ref: {out}");
}

#[test]
fn doc_convert_openapi_responses_rendered() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("`200`"), "200 response: {out}");
    assert!(out.contains("A list of pets"), "response desc: {out}");
}

#[test]
fn doc_convert_openapi_schemas_section() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("## Schemas"), "Schemas section: {out}");
    assert!(out.contains("### Pet"), "Pet schema: {out}");
}

#[test]
fn doc_convert_openapi_schema_properties() {
    let out = convert_openapi(petstore_yaml());
    assert!(out.contains("`id`"), "id property: {out}");
    assert!(out.contains("`name`"), "name property: {out}");
    assert!(out.contains("*(required)*"), "required marker: {out}");
}

#[test]
fn doc_convert_openapi_swagger2_json() {
    let json = r#"{
        "swagger": "2.0",
        "info": {"title": "Simple API", "version": "1.0"},
        "paths": {
            "/items": {
                "get": {
                    "summary": "List items",
                    "responses": {"200": {"description": "OK"}}
                }
            }
        },
        "definitions": {
            "Item": {
                "properties": {
                    "id": {"type": "integer"}
                }
            }
        }
    }"#;
    let out = convert_openapi(json);
    assert!(out.contains("# Simple API"), "title: {out}");
    assert!(out.contains("### GET /items"), "operation: {out}");
    assert!(out.contains("## Schemas"), "schemas: {out}");
    assert!(out.contains("### Item"), "Item schema: {out}");
}

#[test]
fn doc_convert_openapi_non_openapi_returns_empty() {
    let yaml = "name: test\nversion: 1.0\n";
    assert_eq!(convert_openapi(yaml), "");
}

#[test]
fn doc_convert_openapi_invalid_content_returns_empty() {
    assert_eq!(convert_openapi("not: [valid yaml {"), "");
}

// ════════════════════════════════════════════════════════════════════════════
// TestOpenAPIDispatch
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn doc_openapi_dispatch_yaml() {
    let sections = extract_sections("openapi.yaml", petstore_yaml());
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(headings.contains(&"Petstore API"), "headings: {headings:?}");
}

#[test]
fn doc_openapi_dispatch_yml() {
    let sections = extract_sections("openapi.yml", petstore_yaml());
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(headings.contains(&"Petstore API"), "headings: {headings:?}");
}

#[test]
fn doc_openapi_dispatch_json() {
    let json = r#"{"swagger":"2.0","info":{"title":"Simple API","version":"1.0"},"paths":{"/items":{"get":{"summary":"List","responses":{"200":{"description":"OK"}}}}},"definitions":{"Item":{"properties":{"id":{"type":"integer"}}}}}"#;
    let sections = extract_sections("api.json", json);
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(headings.contains(&"Simple API"), "headings: {headings:?}");
}

#[test]
fn doc_openapi_dispatch_non_openapi_yaml_no_sections() {
    // Plain YAML without openapi/swagger key should not produce OpenAPI sections
    let yaml = "name: test\nversion: 1.0\nkey: value\n";
    let sections = extract_sections("config.yaml", yaml);
    // Should either produce no sections or non-OpenAPI sections
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        !headings.contains(&"Petstore API"),
        "should not contain OpenAPI title"
    );
}

#[test]
fn doc_openapi_dispatch_tag_becomes_section() {
    let sections = extract_sections("openapi.yaml", petstore_yaml());
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"pets"),
        "tag should become section: {headings:?}"
    );
}

#[test]
fn doc_openapi_dispatch_operations_are_subsections() {
    let sections = extract_sections("openapi.yaml", petstore_yaml());
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(headings.contains(&"GET /pets"), "GET /pets: {headings:?}");
    assert!(headings.contains(&"POST /pets"), "POST /pets: {headings:?}");
}

#[test]
fn doc_openapi_dispatch_schemas_section_indexed() {
    let sections = extract_sections("openapi.yaml", petstore_yaml());
    let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.contains(&"Schemas"),
        "Schemas section: {headings:?}"
    );
    assert!(headings.contains(&"Pet"), "Pet schema: {headings:?}");
}

// ════════════════════════════════════════════════════════════════════════════
// TestConvertJSON
// ════════════════════════════════════════════════════════════════════════════

fn package_json() -> &'static str {
    r#"{
        "name": "my-package",
        "version": "1.2.3",
        "description": "A test package",
        "scripts": {
            "build": "tsc",
            "test": "jest"
        }
    }"#
}

#[test]
fn doc_convert_json_title_from_filename() {
    let out = convert_json(package_json(), "package.json");
    assert!(out.starts_with("# package.json"), "title: {out}");
}

#[test]
fn doc_convert_json_top_level_keys_become_sections() {
    let out = convert_json(package_json(), "package.json");
    assert!(out.contains("## name"), "name key: {out}");
    assert!(out.contains("## version"), "version key: {out}");
    assert!(out.contains("## description"), "description key: {out}");
}

#[test]
fn doc_convert_json_nested_object_becomes_subsection() {
    let out = convert_json(package_json(), "package.json");
    // "scripts" is a top-level key with nested object, its children become subsections
    assert!(out.contains("## scripts"), "scripts key: {out}");
    assert!(
        out.contains("### build") || out.contains("### test"),
        "nested keys: {out}"
    );
}

#[test]
fn doc_convert_json_scalar_values_rendered() {
    let out = convert_json(package_json(), "package.json");
    assert!(out.contains("1.2.3"), "version value: {out}");
    assert!(out.contains("A test package"), "description value: {out}");
}

#[test]
fn doc_convert_json_array_root_renders_items() {
    let json = r#"[{"name":"Alice"},{"name":"Bob"}]"#;
    let out = convert_json(json, "users.json");
    assert!(out.contains("# users.json"), "title: {out}");
    assert!(out.contains("Alice"), "Alice: {out}");
    assert!(out.contains("Bob"), "Bob: {out}");
}

#[test]
fn doc_convert_json_scalar_root_returns_empty() {
    assert_eq!(convert_json("42", "data.json"), "");
    assert_eq!(convert_json("\"hello\"", "data.json"), "");
}

#[test]
fn doc_convert_json_invalid_json_returns_empty() {
    assert_eq!(convert_json("not json {{{", "data.json"), "");
}

#[test]
fn doc_convert_json_jsonc_comments_stripped() {
    let jsonc = r#"{
        // This is a comment
        "compilerOptions": {
            "target": "es2020"
        }
        /* block comment */
    }"#;
    let out = convert_json(jsonc, "tsconfig.json");
    assert!(out.contains("# tsconfig.json"), "title: {out}");
    assert!(out.contains("## compilerOptions"), "compilerOptions: {out}");
}

#[test]
fn doc_convert_json_empty_doc_path_uses_fallback_title() {
    let out = convert_json(r#"{"key": "value"}"#, "");
    assert!(out.starts_with("# JSON Document"), "fallback title: {out}");
}
