//! Integration tests: doc parser jupyter.

use std::path::Path;

use codesight_retrieval::sections::{
    extract_markdown_sections, extract_sections, preprocess_content, preprocess_notebook,
};

fn fixture_notebook() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/doc_reference/docs/sample.ipynb");
    std::fs::read_to_string(&path).expect("sample.ipynb fixture must exist")
}

/// Helper: preprocess notebook fixture and extract sections from the resulting markdown.
fn fixture_sections() -> (String, Vec<codesight_retrieval::sections::Section>) {
    let content = fixture_notebook();
    let md = preprocess_content(&content, "sample.ipynb");
    let sections = extract_markdown_sections("sample.ipynb", &md);
    (md, sections)
}

// ── TestNotebookParser ports ───────────────────────────────────────────────
#[test]
fn notebook_markdown_cells_included() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# Hello\n\nWorld."]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert!(
        text.contains("# Hello"),
        "expected heading in output: {text}"
    );
    assert!(text.contains("World."), "expected body in output: {text}");
}
#[test]
fn notebook_code_cells_fenced() {
    let nb = r##"{"metadata":{"language_info":{"name":"python"}},"nbformat":4,"cells":[{"cell_type":"code","source":["x = 1"]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert!(text.contains("```python"), "expected python fence: {text}");
    assert!(text.contains("x = 1"), "expected code body: {text}");
    assert!(text.contains("```"), "expected closing fence: {text}");
}
#[test]
fn notebook_kernel_language_detected() {
    let nb = r##"{"metadata":{"kernelspec":{"language":"julia"}},"nbformat":4,"cells":[{"cell_type":"code","source":["println(1)"]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert!(text.contains("```julia"), "expected julia fence: {text}");
}
#[test]
fn notebook_language_defaults_to_python() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"code","source":["x=1"]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert!(
        text.contains("```python"),
        "expected python fence for default language: {text}"
    );
}
#[test]
fn notebook_empty_cells_skipped() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":[""]},{"cell_type":"markdown","source":["# Real"]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert_eq!(text.trim(), "# Real");
}
#[test]
fn notebook_invalid_json_returns_empty() {
    let (text, _) = preprocess_notebook("not json");
    assert_eq!(text, "");
}
#[test]
fn notebook_source_as_list() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# T","itle\n","\nBody."]}]}"##;
    let (text, _) = preprocess_notebook(nb);
    assert!(text.contains("# Title"), "expected joined heading: {text}");
}
#[test]
fn notebook_sections_from_markdown_headings() {
    let (_md, sections) = fixture_sections();
    let titles: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        titles.contains(&"Data Analysis Notebook"),
        "missing 'Data Analysis Notebook': {titles:?}"
    );
    assert!(titles.contains(&"Setup"), "missing 'Setup': {titles:?}");
    assert!(
        titles.contains(&"Loading Data"),
        "missing 'Loading Data': {titles:?}"
    );
    assert!(titles.contains(&"Results"), "missing 'Results': {titles:?}");
}
#[test]
fn notebook_code_in_section_body() {
    let (md, sections) = fixture_sections();
    let setup = sections
        .iter()
        .find(|s| s.heading == "Setup")
        .expect("Setup section must exist");
    let section_content = &md[setup.start_byte..setup.end_byte];
    assert!(
        section_content.contains("```python"),
        "expected python fence in Setup section body: {section_content}"
    );
    assert!(
        section_content.contains("import pandas"),
        "expected 'import pandas' in Setup section body: {section_content}"
    );
}
#[test]
fn notebook_hierarchy_wired() {
    let (_md, sections) = fixture_sections();
    let by_heading: std::collections::HashMap<&str, &codesight_retrieval::sections::Section> =
        sections.iter().map(|s| (s.heading.as_str(), s)).collect();

    let validation = by_heading
        .get("Data Validation")
        .expect("Data Validation section must exist");
    let loading = by_heading
        .get("Loading Data")
        .expect("Loading Data section must exist");

    // Data Validation (h3) should be nested under Loading Data (h2)
    assert_eq!(validation.level, 3, "Data Validation should be h3");
    assert_eq!(loading.level, 2, "Loading Data should be h2");
    // heading_path for Data Validation should include Loading Data as ancestor
    assert!(
        validation.heading_path.contains("Loading Data"),
        "Data Validation heading_path should include Loading Data parent: {}",
        validation.heading_path
    );
}
#[test]
fn notebook_byte_offsets_valid() {
    let (md, sections) = fixture_sections();
    let text_bytes = md.as_bytes();

    assert!(
        !sections.is_empty(),
        "expected at least one section from notebook"
    );

    for sec in &sections {
        assert!(
            sec.start_byte <= sec.end_byte,
            "start_byte {} > end_byte {} for section '{}'",
            sec.start_byte,
            sec.end_byte,
            sec.heading
        );
        assert!(
            sec.end_byte <= text_bytes.len(),
            "end_byte {} > text length {} for section '{}'",
            sec.end_byte,
            text_bytes.len(),
            sec.heading
        );
        let retrieved = std::str::from_utf8(&text_bytes[sec.start_byte..sec.end_byte])
            .expect("valid UTF-8 slice");
        assert!(
            retrieved.contains(&sec.heading),
            "section '{}' heading not found in byte range [{}, {}): {:?}",
            sec.heading,
            sec.start_byte,
            sec.end_byte,
            &retrieved[..retrieved.len().min(200)]
        );
    }
}
#[test]
fn notebook_preprocess_passthrough_for_non_notebook() {
    let md = "# Title\n\nBody.";
    assert_eq!(preprocess_content(md, "doc.md"), md);
    assert_eq!(preprocess_content(md, "doc.rst"), md);
    assert_eq!(preprocess_content(md, "doc.adoc"), md);
}
/// Verifies that extract_sections handles .ipynb files end-to-end.
#[test]
fn notebook_ipynb_dispatch() {
    let nb = r##"{"metadata":{},"nbformat":4,"cells":[{"cell_type":"markdown","source":["# Title\n\nBody."]}]}"##;
    // extract_sections should handle .ipynb directly (raw JSON in, sections out)
    let sections = extract_sections("notebook.ipynb", nb);
    assert!(!sections.is_empty(), "expected sections from .ipynb");
    let titles: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        titles.contains(&"Title"),
        "expected 'Title' in sections: {titles:?}"
    );
}
