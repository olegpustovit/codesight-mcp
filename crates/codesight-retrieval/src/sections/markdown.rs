//! CommonMark heading extraction via `pulldown-cmark`.
//!
//! Hand-rolled ATX / setext detection used to live here. It is gone — the
//! parser handles every CommonMark heading shape, including edge cases the
//! old code mishandled (setext underlines mid-document, hash-style headings
//! with trailing `#`, indented codeblocks that look like underlines).
//!
//! The only Markdown-specific logic left is `detect_frontmatter_skip`,
//! which strips a YAML/TOML frontmatter block at the head of the file
//! before parsing. pulldown-cmark sees `---\nname: x\n---` as a thematic
//! break followed by a paragraph (or, with a trailing thematic break, two
//! breaks bracketing text); without the strip, frontmatter values would
//! still surface as section content. The strip operates at the input
//! boundary; offsets are projected back through it so spans resolve to the
//! original file.

use rustc_hash::FxHashMap;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::Section;
use super::builder::make_summary;

/// Detect a YAML (`---…---`) or TOML (`+++…+++`) frontmatter block at the
/// head of `source` and return `(byte_offset, line_offset)` where the
/// post-frontmatter content begins. `(0, 0)` when no frontmatter is
/// present or the block is unterminated.
fn detect_frontmatter_skip(source: &str) -> (usize, usize) {
    let delim_str = if source.starts_with("---\n") || source.starts_with("---\r\n") {
        "---"
    } else if source.starts_with("+++\n") || source.starts_with("+++\r\n") {
        "+++"
    } else {
        return (0, 0);
    };
    let mut byte_offset = 0usize;
    for (idx, line) in source.lines().enumerate() {
        byte_offset += line.len() + 1;
        if idx == 0 {
            continue;
        }
        if line.trim_end() == delim_str {
            return (byte_offset, idx + 1);
        }
    }
    // Unterminated — fall back to feeding the whole file to the parser.
    // Better to surface noisy sections than silently swallow a document.
    (0, 0)
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Sorted vector of byte positions where each line starts. Index = 0-based
/// line number; value = byte offset of that line's first byte.
fn build_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn byte_to_line(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    }
}

pub fn extract_markdown_sections(file_path: &str, source: &str) -> Vec<Section> {
    let (fm_byte_offset, fm_line_offset) = detect_frontmatter_skip(source);
    let body = &source[fm_byte_offset..];
    if body.is_empty() {
        return Vec::new();
    }

    let line_starts = build_line_starts(body);

    // Phase 1 — walk pulldown-cmark events, capture (level, text, byte_start).
    let mut headings: Vec<(u8, String, usize)> = Vec::new();
    let mut cur_level: Option<u8> = None;
    let mut cur_start: Option<usize> = None;
    let mut cur_text: Option<String> = None;

    for (event, range) in Parser::new_ext(body, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                cur_level = Some(heading_level_to_u8(level));
                cur_start = Some(range.start);
                cur_text = Some(String::new());
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some(buf) = cur_text.as_mut() {
                    buf.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let (Some(level), Some(start), Some(text)) =
                    (cur_level.take(), cur_start.take(), cur_text.take())
                {
                    let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !trimmed.is_empty() {
                        headings.push((level, trimmed, start));
                    }
                }
            }
            _ => {}
        }
    }

    if headings.is_empty() {
        return Vec::new();
    }

    // Phase 2 — breadcrumb paths with sibling dedup (`~N` suffix on collision).
    let mut raw_paths: Vec<String> = Vec::with_capacity(headings.len());
    for (i, &(level, ref text, _)) in headings.iter().enumerate() {
        let mut ancestors = Vec::new();
        let mut walk_level = level;
        for j in (0..i).rev() {
            if headings[j].0 < walk_level {
                ancestors.push(headings[j].1.clone());
                walk_level = headings[j].0;
                if walk_level <= 1 {
                    break;
                }
            }
        }
        ancestors.reverse();
        ancestors.push(text.clone());
        raw_paths.push(ancestors.join(" > "));
    }
    let mut path_counts: FxHashMap<String, usize> = FxHashMap::default();
    let mut final_paths: Vec<String> = Vec::with_capacity(headings.len());
    for path in &raw_paths {
        let count = path_counts.entry(path.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            final_paths.push(path.clone());
        } else {
            final_paths.push(format!("{path}~{count}"));
        }
    }

    // Phase 3 — section spans: each heading's start to the next heading's
    // start (or EOF). Lines and bytes are projected back through the
    // frontmatter so spans resolve to positions in the original file.
    let body_total_bytes = body.len();
    let mut sections = Vec::with_capacity(headings.len());

    for i in 0..headings.len() {
        let (level, ref text, body_start_byte) = headings[i];
        let body_end_byte = if i + 1 < headings.len() {
            headings[i + 1].2
        } else {
            body_total_bytes
        };

        let body_start_line = byte_to_line(&line_starts, body_start_byte);
        let last_byte = body_end_byte.saturating_sub(1).max(body_start_byte);
        let mut end_line_idx = byte_to_line(&line_starts, last_byte);

        // Trim trailing blank lines so `end_line` lands on the last
        // non-empty content line of the section.
        while end_line_idx > body_start_line {
            let line_start = line_starts[end_line_idx];
            let line_end = line_starts
                .get(end_line_idx + 1)
                .copied()
                .unwrap_or(body_total_bytes);
            if body[line_start..line_end].trim().is_empty() {
                end_line_idx -= 1;
            } else {
                break;
            }
        }

        let heading_path = &final_paths[i];
        let section_text = &body[body_start_byte..body_end_byte.min(body_total_bytes)];
        let content_hash = blake3::hash(section_text.as_bytes()).to_hex().to_string();
        // Summary derives from the post-heading body, not the heading line
        // itself (matches the previous semantics).
        let content_only = section_text
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or("");
        let summary = make_summary(content_only);

        let level_str = format!("h{level}");
        let id = format!("{file_path}::{heading_path}#{level_str}");

        sections.push(Section {
            id,
            file_path: file_path.to_string(),
            heading: text.clone(),
            heading_path: heading_path.clone(),
            level,
            start_line: ((body_start_line + 1) + fm_line_offset) as u32,
            end_line: ((end_line_idx + 1) + fm_line_offset) as u32,
            start_byte: body_start_byte + fm_byte_offset,
            end_byte: body_end_byte.min(body_total_bytes) + fm_byte_offset,
            content_hash,
            summary,
        });
    }

    sections
}
