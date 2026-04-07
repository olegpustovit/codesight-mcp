//! AsciiDoc `=` heading extraction.

use super::Section;
use super::builder::build_sections;

/// Extract sections from an AsciiDoc document.
///
/// AsciiDoc headings use leading `=` characters: `= H1`, `== H2`, `=== H3`, etc.
/// Block delimiters like `----` must not be confused with headings.
pub fn extract_asciidoc_sections(file_path: &str, source: &str) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut headings: Vec<(String, usize, usize)> = Vec::new();
    let mut levels: Vec<u8> = Vec::new();

    let mut byte_offset = 0;
    for (line_idx, line) in lines.iter().enumerate() {
        if let Some((level, text)) = parse_asciidoc_heading(line) {
            headings.push((text, line_idx, byte_offset));
            levels.push(level);
        }
        byte_offset += line.len() + 1;
    }

    if headings.is_empty() {
        return Vec::new();
    }

    build_sections(file_path, source, &lines, &headings, &levels)
}

fn parse_asciidoc_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('=') {
        return None;
    }
    let eqs = trimmed.bytes().take_while(|&b| b == b'=').count();
    if eqs == 0 || eqs > 6 {
        return None;
    }
    let rest = &trimmed[eqs..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some((eqs as u8, text))
}
