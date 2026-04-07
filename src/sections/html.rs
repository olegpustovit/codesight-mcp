//! HTML to markdown-like conversion for section extraction.

use super::Section;
use super::markdown::extract_markdown_sections;

/// Convert HTML to markdown-like text suitable for section extraction.
///
/// Strips `<script>`, `<style>`, `<nav>`, `<footer>` elements entirely.
/// Converts `<h1>`-`<h6>` to ATX headings (`# ` .. `###### `).
/// Converts `<pre><code>` blocks to fenced code blocks.
/// Decodes HTML character references (`&amp;` → `&`, `&#39;` → `'`, etc.).
/// Preserves `<p>` paragraph text.
pub fn convert_html_to_markdown(html: &str) -> String {
    // Phase 1: Strip elements we want to remove entirely (script, style, nav, footer)
    let stripped = strip_html_elements(html, &["script", "style", "nav", "footer", "head"]);

    // Phase 2: Convert to markdown-like text
    let mut result = String::with_capacity(stripped.len());
    let mut chars = stripped.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Collect the tag
            let mut tag_content = String::new();
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
                tag_content.push(c);
            }
            let tag_lower = tag_content.to_ascii_lowercase();
            let tag_name = tag_lower.split_whitespace().next().unwrap_or("");

            // Handle heading tags
            if let Some(level) = parse_heading_tag(tag_name) {
                // Collect text until closing tag
                let closing = format!("</h{level}>");
                let inner = collect_until_closing(&mut chars, &closing);
                let text = strip_all_tags(&decode_html_entities(&inner));
                let text = text.trim();
                if !text.is_empty() {
                    let hashes = "#".repeat(level as usize);
                    result.push_str(&format!("\n{hashes} {text}\n\n"));
                }
            } else if tag_name == "pre" {
                // Collect until </pre>
                let inner = collect_until_closing(&mut chars, "</pre>");
                // Strip <code> and </code> tags inside
                let code = strip_all_tags(&inner);
                let code = decode_html_entities(&code);
                result.push_str(&format!("\n```\n{code}\n```\n\n"));
            } else if tag_name == "p" {
                let inner = collect_until_closing(&mut chars, "</p>");
                let text = strip_all_tags(&decode_html_entities(&inner));
                let text = text.trim();
                if !text.is_empty() {
                    result.push_str(text);
                    result.push_str("\n\n");
                }
            } else if tag_name.starts_with('/') {
                // Closing tag — skip
            }
            // Other tags are ignored (their text content falls through)
        } else {
            result.push(ch);
        }
    }

    result
}

/// Strip named HTML elements and all their content.
fn strip_html_elements(html: &str, tags: &[&str]) -> String {
    let mut result = html.to_string();
    for tag in tags {
        loop {
            let open_pattern = format!("<{}", tag);
            let close_pattern = format!("</{}>", tag);
            let lower = result.to_ascii_lowercase();
            if let Some(start) = lower.find(&open_pattern) {
                if let Some(end_rel) = lower[start..].find(&close_pattern) {
                    let end = start + end_rel + close_pattern.len();
                    result.replace_range(start..end, "");
                } else {
                    // No closing tag — strip from open tag to end of its > or end of string
                    if let Some(gt) = result[start..].find('>') {
                        result.replace_range(start..start + gt + 1, "");
                    } else {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }
    result
}

/// Parse heading tag name like "h1", "h2", etc. Returns the level.
fn parse_heading_tag(tag: &str) -> Option<u8> {
    if tag.len() == 2 && tag.starts_with('h') {
        let level = tag.as_bytes()[1].wrapping_sub(b'0');
        if (1..=6).contains(&level) {
            return Some(level);
        }
    }
    None
}

/// Collect characters until a case-insensitive closing tag is found.
fn collect_until_closing(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    closing: &str,
) -> String {
    let mut buf = String::new();
    let closing_lower = closing.to_ascii_lowercase();
    while chars.peek().is_some() {
        let ch = chars.next().unwrap();
        buf.push(ch);
        if buf.len() >= closing_lower.len() {
            let tail: String = buf[buf.len() - closing_lower.len()..].to_ascii_lowercase();
            if tail == closing_lower {
                // Remove the closing tag from buf
                buf.truncate(buf.len() - closing_lower.len());
                break;
            }
        }
    }
    buf
}

/// Strip all HTML tags from text, leaving only text content.
fn strip_all_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
}

/// Decode HTML character references (named and numeric).
fn decode_html_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            let mut entity = String::new();
            let mut found_semi = false;
            for c in chars.by_ref() {
                if c == ';' {
                    found_semi = true;
                    break;
                }
                entity.push(c);
                if entity.len() > 10 {
                    break;
                }
            }
            if found_semi {
                if let Some(decoded) = decode_entity(&entity) {
                    result.push(decoded);
                } else {
                    result.push('&');
                    result.push_str(&entity);
                    result.push(';');
                }
            } else {
                result.push('&');
                result.push_str(&entity);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn decode_entity(entity: &str) -> Option<char> {
    // Numeric references
    if entity.starts_with('#') {
        let num_str = &entity[1..];
        if let Some(hex_str) = num_str
            .strip_prefix('x')
            .or_else(|| num_str.strip_prefix('X'))
        {
            u32::from_str_radix(hex_str, 16)
                .ok()
                .and_then(char::from_u32)
        } else {
            num_str.parse::<u32>().ok().and_then(char::from_u32)
        }
    } else {
        // Named references (common ones)
        match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{00A0}'),
            "eacute" => Some('\u{00E9}'),
            "Eacute" => Some('\u{00C9}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "ldquo" => Some('\u{201C}'),
            "rdquo" => Some('\u{201D}'),
            "copy" => Some('\u{00A9}'),
            "reg" => Some('\u{00AE}'),
            "trade" => Some('\u{2122}'),
            "hellip" => Some('\u{2026}'),
            "bull" => Some('\u{2022}'),
            "middot" => Some('\u{00B7}'),
            "laquo" => Some('\u{00AB}'),
            "raquo" => Some('\u{00BB}'),
            "frac12" => Some('\u{00BD}'),
            "frac14" => Some('\u{00BC}'),
            "frac34" => Some('\u{00BE}'),
            "deg" => Some('\u{00B0}'),
            "micro" => Some('\u{00B5}'),
            "para" => Some('\u{00B6}'),
            "sect" => Some('\u{00A7}'),
            "uml" => Some('\u{00A8}'),
            "agrave" => Some('\u{00E0}'),
            "egrave" => Some('\u{00E8}'),
            "igrave" => Some('\u{00EC}'),
            "ograve" => Some('\u{00F2}'),
            "ugrave" => Some('\u{00F9}'),
            "aacute" => Some('\u{00E1}'),
            "iacute" => Some('\u{00ED}'),
            "oacute" => Some('\u{00F3}'),
            "uacute" => Some('\u{00FA}'),
            "acirc" => Some('\u{00E2}'),
            "ecirc" => Some('\u{00EA}'),
            "icirc" => Some('\u{00EE}'),
            "ocirc" => Some('\u{00F4}'),
            "ucirc" => Some('\u{00FB}'),
            "atilde" => Some('\u{00E3}'),
            "ntilde" => Some('\u{00F1}'),
            "otilde" => Some('\u{00F5}'),
            "auml" => Some('\u{00E4}'),
            "euml" => Some('\u{00EB}'),
            "iuml" => Some('\u{00EF}'),
            "ouml" => Some('\u{00F6}'),
            "uuml" => Some('\u{00FC}'),
            "ccedil" => Some('\u{00E7}'),
            "szlig" => Some('\u{00DF}'),
            _ => None,
        }
    }
}

/// Extract sections from an HTML document by converting to markdown first.
pub fn extract_html_sections(file_path: &str, source: &str) -> Vec<Section> {
    let markdown = convert_html_to_markdown(source);
    extract_markdown_sections(file_path, &markdown)
}
