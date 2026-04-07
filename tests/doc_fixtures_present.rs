//! Sanity checks for the checked-in documentation fixtures.

use codesight_mcp::test_support::doc_reference_fixtures_dir;

#[test]
fn doc_reference_fixtures_mirrored() {
    let root = doc_reference_fixtures_dir();
    assert!(
        root.join("docs/sample.md").is_file(),
        "missing {:?}",
        root.join("docs/sample.md")
    );
    assert!(
        root.join("docs/nested/guide.md").is_file(),
        "missing nested guide"
    );
    assert!(
        root.join("text/sample.txt").is_file(),
        "missing text sample"
    );
}

#[test]
fn sample_md_has_expected_sections() {
    let sample = std::fs::read_to_string(doc_reference_fixtures_dir().join("docs/sample.md"))
        .expect("sample.md fixture");
    assert!(sample.contains("# Sample Documentation"));
    assert!(sample.contains("## Installation"));
    assert!(sample.contains("## Usage"));
}
