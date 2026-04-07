//! Markdown/RST/text section extraction via heading hierarchy.
//!
//! Sections are bounded by heading level: an h2 section runs from its heading
//! to the next h2 (or h1, or EOF). Nested headings (h3 inside h2) become child
//! sections with breadcrumb-style stable IDs.

mod asciidoc;
mod builder;
mod html;
mod markdown;
mod openapi_json;
mod paths;
mod plain;
mod preprocess;
mod refs;
mod rst;

pub use asciidoc::extract_asciidoc_sections;
pub use html::{convert_html_to_markdown, extract_html_sections};
pub use markdown::extract_markdown_sections;
pub use openapi_json::{convert_json, convert_openapi, sniff_openapi};
pub use paths::{is_doc_file, should_skip_relative_doc_path};
pub use plain::extract_text_sections;
pub use preprocess::{preprocess_content, preprocess_notebook};
pub use refs::{extract_references, extract_tags};
pub use rst::extract_rst_sections;

/// A parsed documentation section.
#[derive(Debug, Clone)]
pub struct Section {
    pub id: String,
    pub file_path: String,
    pub heading: String,
    pub heading_path: String,
    pub level: u8,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub content_hash: String,
    pub summary: String,
}

/// Dispatch to the right section extractor based on file extension.
///
/// If the native parser for a format returns no sections, falls back to markdown
/// ATX/setext parsing (since many `.rst` or `.adoc` files use ATX `#` headings
/// in practice).  For `.txt`, further falls back to paragraph splitting.
/// For `.ipynb`, preprocesses notebook JSON into Markdown first.
pub fn extract_sections(file_path: &str, source: &str) -> Vec<Section> {
    let lower = file_path.to_ascii_lowercase();
    if lower.ends_with(".ipynb") {
        let (md, _lang) = preprocess_notebook(source);
        if md.is_empty() {
            return Vec::new();
        }
        return extract_markdown_sections(file_path, &md);
    }
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        return extract_html_sections(file_path, source);
    }
    if (lower.ends_with(".yaml") || lower.ends_with(".yml") || lower.ends_with(".json"))
        && sniff_openapi(source, file_path)
    {
        let md = convert_openapi(source);
        if !md.is_empty() {
            return extract_markdown_sections(file_path, &md);
        }
    }
    if lower.ends_with(".json") {
        let md = convert_json(source, file_path);
        if !md.is_empty() {
            return extract_markdown_sections(file_path, &md);
        }
    }
    if lower.ends_with(".rst") {
        let native = extract_rst_sections(file_path, source);
        if native.is_empty() {
            extract_markdown_sections(file_path, source)
        } else {
            native
        }
    } else if lower.ends_with(".adoc") || lower.ends_with(".asciidoc") {
        let native = extract_asciidoc_sections(file_path, source);
        if native.is_empty() {
            extract_markdown_sections(file_path, source)
        } else {
            native
        }
    } else if lower.ends_with(".txt") {
        let md = extract_markdown_sections(file_path, source);
        if md.is_empty() {
            extract_text_sections(file_path, source)
        } else {
            md
        }
    } else {
        extract_markdown_sections(file_path, source)
    }
}

#[cfg(test)]
mod tests;
