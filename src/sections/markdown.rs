//! ATX and setext markdown heading extraction.

use std::collections::{HashMap, HashSet};

use super::Section;
use super::builder::make_summary;

pub fn extract_markdown_sections(file_path: &str, source: &str) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    // Phase 1: Find all headings with their positions (ATX + setext)
    let mut headings: Vec<(u8, String, usize, usize)> = Vec::new(); // (level, text, line_idx, byte_offset)
    let mut byte_offsets: Vec<usize> = Vec::with_capacity(lines.len());
    {
        let mut off = 0;
        for line in &lines {
            byte_offsets.push(off);
            off += line.len() + 1;
        }
    }
    let mut setext_underline_indices: HashSet<usize> = HashSet::new();

    // Pre-scan for setext headings (text line followed by === or --- line)
    for line_idx in 1..lines.len() {
        if let Some(level) = parse_setext_underline(lines[line_idx]) {
            let text_line = lines[line_idx - 1].trim();
            if !text_line.is_empty()
                && !text_line.starts_with('#')
                && !setext_underline_indices.contains(&(line_idx - 1))
            {
                let text = text_line.split_whitespace().collect::<Vec<_>>().join(" ");
                headings.push((level, text, line_idx - 1, byte_offsets[line_idx - 1]));
                setext_underline_indices.insert(line_idx);
            }
        }
    }

    // ATX headings (skip lines already consumed as setext text or underlines)
    for (line_idx, line) in lines.iter().enumerate() {
        if setext_underline_indices.contains(&line_idx)
            || (line_idx > 0 && setext_underline_indices.contains(&(line_idx + 1)))
        {
            continue;
        }
        // Also skip if this line is a setext text line (line_idx is text, line_idx+1 is underline)
        if line_idx + 1 < lines.len() && setext_underline_indices.contains(&(line_idx + 1)) {
            continue;
        }
        if let Some((level, text)) = parse_atx_heading(line) {
            headings.push((level, text, line_idx, byte_offsets[line_idx]));
        }
    }

    // Sort by position since we collected setext first, then ATX
    headings.sort_by_key(|h| h.2);

    if headings.is_empty() {
        return Vec::new();
    }

    // Phase 2: Assign each heading its breadcrumb path, handling dedup for siblings
    // First pass: compute raw paths (without dedup suffixes)
    let mut raw_paths: Vec<String> = Vec::new();
    for (i, &(level, ref text, _, _)) in headings.iter().enumerate() {
        let mut ancestors = Vec::new();
        let mut cur_level = level;
        for j in (0..i).rev() {
            if headings[j].0 < cur_level {
                ancestors.push(headings[j].1.clone());
                cur_level = headings[j].0;
                if cur_level <= 1 {
                    break;
                }
            }
        }
        ancestors.reverse();
        ancestors.push(text.clone());
        raw_paths.push(ancestors.join(" > "));
    }

    // Second pass: detect duplicates and add ~N suffixes
    let mut path_counts: HashMap<String, usize> = HashMap::new();
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

    // Phase 3: Build sections
    let total_bytes = source.len();
    let total_lines = lines.len();
    let mut sections = Vec::new();

    for (i, &(level, ref text, line_idx, start_byte)) in headings.iter().enumerate() {
        let end_line_idx = if i + 1 < headings.len() {
            let next_line = headings[i + 1].2;
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
            headings[i + 1].3
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
        let section_text = &source[start_byte..end_byte.min(total_bytes)];
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
            start_byte,
            end_byte: end_byte.min(total_bytes),
            content_hash,
            summary,
        });
    }

    sections
}

fn parse_atx_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_string();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    Some((hashes as u8, text))
}

fn parse_setext_underline(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first = trimmed.as_bytes()[0];
    if first == b'=' {
        if trimmed.bytes().all(|b| b == b'=') {
            return Some(1);
        }
    } else if first == b'-' && trimmed.bytes().all(|b| b == b'-') {
        return Some(2);
    }
    None
}
