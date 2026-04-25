//! HTML to markdown conversion for section extraction.
//!
//! Uses `htmd` (built on `html5ever`) for spec-compliant HTML parsing,
//! entity decoding, and ATX-heading + fenced-code-block emission.

use htmd::HtmlToMarkdown;

use super::Section;
use super::markdown::extract_markdown_sections;

/// Convert HTML to markdown-like text suitable for section extraction.
///
/// Strips chrome elements (`<script>`, `<style>`, `<nav>`, `<footer>`, `<head>`)
/// and lets `htmd` handle headings, fenced code, paragraphs, and HTML entity
/// decoding.
pub fn convert_html_to_markdown(html: &str) -> String {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "nav", "footer", "head"])
        .build();
    converter.convert(html).unwrap_or_default()
}

/// Extract sections from an HTML document by converting to markdown first.
pub fn extract_html_sections(file_path: &str, source: &str) -> Vec<Section> {
    let markdown = convert_html_to_markdown(source);
    extract_markdown_sections(file_path, &markdown)
}
