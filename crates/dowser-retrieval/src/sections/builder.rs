//! Shared section assembly for RST/AsciiDoc and summary truncation.

use rustc_hash::FxHashMap;

use super::Section;

pub(crate) fn make_summary(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() <= 200 {
        return trimmed.to_string();
    }
    let mut end = 200;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = &trimmed[..end];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &truncated[..pos]),
        None => format!("{truncated}..."),
    }
}

pub(crate) fn slug_from_text(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Shared section-building logic for RST/AsciiDoc parsers.
pub(crate) fn build_sections(
    file_path: &str,
    source: &str,
    lines: &[&str],
    headings: &[(String, usize, usize)], // (text, line_idx, byte_offset)
    levels: &[u8],
) -> Vec<Section> {
    let total_bytes = source.len();
    let total_lines = lines.len();

    let mut byte_offsets: Vec<usize> = Vec::with_capacity(lines.len() + 1);
    {
        let mut off = 0;
        for line in lines {
            byte_offsets.push(off);
            off += line.len() + 1;
        }
        byte_offsets.push(off);
    }

    let mut raw_paths: Vec<String> = Vec::new();
    for (i, (text, _, _)) in headings.iter().enumerate() {
        let level = levels[i];
        let mut ancestors = Vec::new();
        let mut cur_level = level;
        for j in (0..i).rev() {
            if levels[j] < cur_level {
                ancestors.push(headings[j].0.clone());
                cur_level = levels[j];
                if cur_level <= 1 {
                    break;
                }
            }
        }
        ancestors.reverse();
        ancestors.push(text.clone());
        raw_paths.push(ancestors.join(" > "));
    }

    let mut path_counts: FxHashMap<String, usize> = FxHashMap::default();
    let mut final_paths: Vec<String> = Vec::new();
    for path in &raw_paths {
        let count = path_counts.entry(path.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            final_paths.push(path.clone());
        } else {
            final_paths.push(format!("{path}~{count}"));
        }
    }

    let mut sections = Vec::new();
    for (i, (text, line_idx, start_byte)) in headings.iter().enumerate() {
        let level = levels[i];
        let end_line_idx = if i + 1 < headings.len() {
            let next_line = headings[i + 1].1;
            let mut end = next_line;
            while end > line_idx + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            end
        } else {
            let mut end = total_lines;
            while end > line_idx + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            end
        };

        let end_byte = if i + 1 < headings.len() {
            headings[i + 1].2
        } else {
            total_bytes
        };

        let heading_path = &final_paths[i];

        let content_start = if line_idx + 1 < total_lines {
            lines[line_idx + 1..end_line_idx].join("\n")
        } else {
            String::new()
        };
        let summary = make_summary(&content_start);
        let section_text = &source[*start_byte..end_byte.min(total_bytes)];
        let content_hash = blake3::hash(section_text.as_bytes()).to_hex().to_string();

        let level_str = format!("h{level}");
        let id = format!("{file_path}::{heading_path}#{level_str}");

        sections.push(Section {
            id,
            file_path: file_path.to_string(),
            heading: text.clone(),
            heading_path: heading_path.clone(),
            level,
            start_line: (line_idx + 1) as u32,
            end_line: end_line_idx as u32,
            start_byte: *start_byte,
            end_byte: end_byte.min(total_bytes),
            content_hash,
            summary,
        });
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::make_summary;

    #[test]
    fn truncates_on_char_boundary_with_multibyte_at_limit() {
        let mut s = "a".repeat(198);
        s.push('…');
        s.push_str(" tail");
        let out = make_summary(&s);
        assert!(out.ends_with("..."));
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn short_input_returned_verbatim() {
        assert_eq!(make_summary("hello"), "hello");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(make_summary("   "), "");
    }
}
