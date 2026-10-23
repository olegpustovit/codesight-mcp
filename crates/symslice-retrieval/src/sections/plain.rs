//! Plain text: paragraph-based pseudo-sections.

use rustc_hash::FxHashMap;

use super::Section;
use super::builder::{make_summary, slug_from_text};

/// Extract sections from a plain text document by splitting on blank-line-separated paragraphs.
pub fn extract_text_sections(file_path: &str, source: &str) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut paragraphs: Vec<(usize, usize)> = Vec::new();
    let mut para_start: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if let Some(start) = para_start.take() {
                paragraphs.push((start, i));
            }
        } else if para_start.is_none() {
            para_start = Some(i);
        }
    }
    if let Some(start) = para_start {
        paragraphs.push((start, lines.len()));
    }

    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut byte_offsets: Vec<usize> = Vec::with_capacity(lines.len() + 1);
    {
        let mut off = 0;
        for line in &lines {
            byte_offsets.push(off);
            off += line.len() + 1;
        }
        byte_offsets.push(off);
    }

    let total_bytes = source.len();
    let mut sections = Vec::new();
    let mut id_counts: FxHashMap<String, usize> = FxHashMap::default();

    for (idx, &(start, end)) in paragraphs.iter().enumerate() {
        let first_line = lines[start].trim();
        let heading = if first_line.len() <= 60 {
            first_line.to_string()
        } else {
            match first_line[..60].rfind(' ') {
                Some(pos) => format!("{}...", &first_line[..pos]),
                None => format!("{}...", &first_line[..60]),
            }
        };

        let heading_path = heading.clone();
        let slug = slug_from_text(&heading);
        let count = id_counts.entry(slug.clone()).or_insert(0);
        *count += 1;
        let id_suffix = if *count > 1 {
            format!("~{count}")
        } else {
            String::new()
        };
        let id = format!("{file_path}::{heading_path}{id_suffix}#p{}", idx + 1);

        let start_byte = byte_offsets[start];
        let end_byte = if end < byte_offsets.len() {
            byte_offsets[end].min(total_bytes)
        } else {
            total_bytes
        };

        let section_text = &source[start_byte..end_byte.min(total_bytes)];
        let content_hash = blake3::hash(section_text.as_bytes()).to_hex().to_string();

        let content_for_summary = lines[start..end].join("\n");
        let summary = make_summary(&content_for_summary);

        sections.push(Section {
            id,
            file_path: file_path.to_string(),
            heading,
            heading_path,
            level: 1,
            start_line: (start + 1) as u32,
            end_line: end as u32,
            start_byte,
            end_byte: end_byte.min(total_bytes),
            content_hash,
            summary,
        });
    }

    sections
}
