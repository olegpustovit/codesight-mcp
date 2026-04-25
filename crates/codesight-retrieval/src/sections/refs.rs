//! URL and hashtag tag extraction from documentation text.

use rustc_hash::FxHashSet;

pub fn extract_references(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = FxHashSet::default();
    // helper closure
    macro_rules! insert {
        ($url:expr) => {
            if seen.insert($url.to_string()) {
                result.push($url.to_string());
            }
        };
    }
    // We'll collect into result with dedup via seen
    let _ = &seen; // suppress unused

    // Markdown inline links: [text](url)
    {
        let mut rest = text;
        while let Some(open) = rest.find("](") {
            let after = &rest[open + 2..];
            if let Some(close) = after.find(')') {
                let url = after[..close].trim();
                if !url.is_empty() {
                    insert!(url);
                }
                rest = &after[close + 1..];
            } else {
                break;
            }
        }
    }

    // Reference-style links: [label]: url  (at start of line)
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[')
            && let Some(bracket_end) = trimmed.find("]:")
        {
            let url = trimmed[bracket_end + 2..].trim();
            let url = url.split_whitespace().next().unwrap_or("");
            if !url.is_empty() {
                insert!(url);
            }
        }
        // RST reference: .. _label: url
        if trimmed.starts_with(".. _")
            && let Some(colon) = trimmed[4..].find(':')
        {
            let url = trimmed[4 + colon + 1..].trim();
            if !url.is_empty() {
                insert!(url);
            }
        }
    }

    // Bare URLs: https://... or http://...
    for word in text.split_whitespace() {
        // Strip trailing punctuation
        let cleaned = word.trim_end_matches([')', '>', ',', '.', ';']);
        if cleaned.starts_with("https://") || cleaned.starts_with("http://") {
            insert!(cleaned);
        }
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// extract_tags — extract hashtag-style tags from content
// ─────────────────────────────────────────────────────────────────────────────

/// Extract `#tag` style tags from content text.
///
/// A tag is a `#` followed by one or more alphanumeric/underscore characters,
/// not inside a markdown heading line (lines starting with `#`).
/// Also extracts tags from YAML frontmatter `tags: [tag1, tag2]` and
/// HTML comments `<!-- tags: tag1, tag2 -->`.
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut seen = FxHashSet::default();

    // Hashtag-style tags in body text (skip heading lines)
    for line in text.lines() {
        let trimmed = line.trim();
        // Skip markdown heading lines
        if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
            continue;
        }
        let mut chars = trimmed.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch == '#' {
                // Must not be at position 0 of a non-heading line starting with #
                // Check that the previous char is whitespace or start of line
                let prev_ok = i == 0
                    || trimmed
                        .as_bytes()
                        .get(i - 1)
                        .map(|b| b.is_ascii_whitespace())
                        .unwrap_or(true);
                if prev_ok {
                    let tag_start = i + 1;
                    let mut tag_end = tag_start;
                    while let Some(&(j, c)) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            tag_end = j + c.len_utf8();
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if tag_end > tag_start {
                        let tag = &trimmed[tag_start..tag_end];
                        if seen.insert(tag.to_string()) {
                            tags.push(tag.to_string());
                        }
                    }
                }
            }
        }
    }

    // YAML frontmatter tags: [tag1, tag2]
    if text.starts_with("---")
        && let Some(end) = text[3..].find("---")
    {
        let frontmatter = &text[3..3 + end];
        for line in frontmatter.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("tags:") {
                let rest = rest.trim();
                // Parse [tag1, tag2] or tag1, tag2
                let rest = rest.trim_start_matches('[').trim_end_matches(']');
                for tag in rest.split(',') {
                    let tag = tag.trim().trim_matches('"').trim_matches('\'');
                    if !tag.is_empty() && seen.insert(tag.to_string()) {
                        tags.push(tag.to_string());
                    }
                }
            }
        }
    }

    // HTML comment tags: <!-- tags: tag1, tag2 -->
    let mut rest = text;
    while let Some(start) = rest.find("<!-- tags:") {
        let after = &rest[start + 10..];
        if let Some(end) = after.find("-->") {
            let tag_str = after[..end].trim();
            for tag in tag_str.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() && seen.insert(tag.to_string()) {
                    tags.push(tag.to_string());
                }
            }
            rest = &after[end + 3..];
        } else {
            break;
        }
    }

    tags
}
