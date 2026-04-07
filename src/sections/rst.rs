//! reStructuredText heading extraction via adornment lines.

use std::collections::HashSet;

use super::Section;
use super::builder::build_sections;

/// Extract sections from an RST document.
///
/// RST headings are formed by underlining (and optionally overlining) text with
/// a punctuation character. The adornment style (char + has_overline) seen first
/// is level 1, second is level 2, etc.
pub fn extract_rst_sections(file_path: &str, source: &str) -> Vec<Section> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let rst_adornment_chars: &[u8] = b"=!\"#$%&'()*+,-./:;<>?@[\\]^_`{|}~";

    fn is_adornment_line(line: &str, chars: &[u8]) -> Option<u8> {
        let trimmed = line.trim_end();
        if trimmed.len() < 1 {
            return None;
        }
        let first = trimmed.as_bytes()[0];
        if !chars.contains(&first) {
            return None;
        }
        if trimmed.bytes().all(|b| b == first) {
            Some(first)
        } else {
            None
        }
    }

    let mut headings: Vec<(String, usize, usize)> = Vec::new();
    let mut adornment_styles: Vec<(u8, bool)> = Vec::new();
    let mut heading_style_indices: Vec<usize> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    let mut byte_offsets: Vec<usize> = Vec::with_capacity(lines.len());
    {
        let mut off = 0;
        for line in &lines {
            byte_offsets.push(off);
            off += line.len() + 1;
        }
    }

    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len() {
            if let Some(ch) = is_adornment_line(lines[i], rst_adornment_chars) {
                let text = lines[i + 1].trim();
                if !text.is_empty() {
                    if let Some(ch2) = is_adornment_line(lines[i + 2], rst_adornment_chars) {
                        if ch == ch2
                            && lines[i].trim_end().len() >= text.len()
                            && lines[i + 2].trim_end().len() >= text.len()
                        {
                            let style = (ch, true);
                            let style_idx = match adornment_styles.iter().position(|s| *s == style)
                            {
                                Some(idx) => idx,
                                None => {
                                    adornment_styles.push(style);
                                    adornment_styles.len() - 1
                                }
                            };
                            headings.push((text.to_string(), i + 1, byte_offsets[i + 1]));
                            heading_style_indices.push(style_idx);
                            consumed.insert(i);
                            consumed.insert(i + 1);
                            consumed.insert(i + 2);
                            i += 3;
                            continue;
                        }
                    }
                }
            }
        }

        if i + 1 < lines.len() && !consumed.contains(&i) {
            let text = lines[i].trim();
            if !text.is_empty() {
                if let Some(ch) = is_adornment_line(lines[i + 1], rst_adornment_chars) {
                    if lines[i + 1].trim_end().len() >= text.len() {
                        let style = (ch, false);
                        let style_idx = match adornment_styles.iter().position(|s| *s == style) {
                            Some(idx) => idx,
                            None => {
                                adornment_styles.push(style);
                                adornment_styles.len() - 1
                            }
                        };
                        headings.push((text.to_string(), i, byte_offsets[i]));
                        heading_style_indices.push(style_idx);
                        consumed.insert(i);
                        consumed.insert(i + 1);
                        i += 2;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    if headings.is_empty() {
        return Vec::new();
    }

    let levels: Vec<u8> = heading_style_indices
        .iter()
        .map(|idx| (*idx as u8) + 1)
        .collect();

    build_sections(file_path, source, &lines, &headings, &levels)
}
