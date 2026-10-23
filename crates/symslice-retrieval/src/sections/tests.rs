use super::{extract_markdown_sections, should_skip_relative_doc_path};

#[test]
fn extracts_markdown_headings() {
    let source = r#"# Getting Started

Welcome to the project.

## Installation

Run `pip install mylib`.

### From Source

Clone and build:

```
make install
```

## Configuration

Set env vars.

## Configuration

Second config section.
"#;
    let sections = extract_markdown_sections("docs/readme.md", source);
    assert_eq!(sections.len(), 5);

    assert_eq!(sections[0].heading, "Getting Started");
    assert_eq!(sections[0].heading_path, "Getting Started");
    assert_eq!(sections[0].id, "docs/readme.md::Getting Started#h1");
    assert_eq!(sections[0].level, 1);

    assert_eq!(sections[1].heading, "Installation");
    assert_eq!(sections[1].heading_path, "Getting Started > Installation");
    assert_eq!(
        sections[1].id,
        "docs/readme.md::Getting Started > Installation#h2"
    );

    assert_eq!(sections[2].heading, "From Source");
    assert_eq!(
        sections[2].heading_path,
        "Getting Started > Installation > From Source"
    );

    assert_eq!(sections[3].heading, "Configuration");
    assert_eq!(sections[3].heading_path, "Getting Started > Configuration");

    assert_eq!(sections[4].heading, "Configuration");
    assert_eq!(
        sections[4].heading_path,
        "Getting Started > Configuration~2"
    );
}

#[test]
fn should_skip_matches_doc_fixture_expectations() {
    assert!(should_skip_relative_doc_path("build/output.md"));
    assert!(should_skip_relative_doc_path("node_modules/pkg/README.md"));
    assert!(!should_skip_relative_doc_path("rebuild/output.md"));
    assert!(!should_skip_relative_doc_path("docs/guide.md"));
    assert!(should_skip_relative_doc_path("submodule/.git/config"));
    assert!(!should_skip_relative_doc_path("docs/build_notes.md"));
}

#[test]
fn section_boundaries_correct() {
    let source =
        "# Title\n\nIntro text.\n\n## Section A\n\nContent A.\n\n## Section B\n\nContent B.\n";
    let sections = extract_markdown_sections("doc.md", source);
    assert_eq!(sections.len(), 3);
    assert!(sections[0].summary.contains("Intro text"));
    assert!(sections[1].summary.contains("Content A"));
    assert!(sections[2].summary.contains("Content B"));
}
