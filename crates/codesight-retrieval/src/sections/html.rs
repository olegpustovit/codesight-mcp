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
    for ch in chars.by_ref() {
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

static NAMED_ENTITIES: phf::Map<&'static str, char> = phf::phf_map! {
    "amp" => '&',
    "lt" => '<',
    "gt" => '>',
    "quot" => '"',
    "apos" => '\'',
    "nbsp" => '\u{00A0}',
    "eacute" => '\u{00E9}',
    "Eacute" => '\u{00C9}',
    "mdash" => '\u{2014}',
    "ndash" => '\u{2013}',
    "lsquo" => '\u{2018}',
    "rsquo" => '\u{2019}',
    "ldquo" => '\u{201C}',
    "rdquo" => '\u{201D}',
    "copy" => '\u{00A9}',
    "reg" => '\u{00AE}',
    "trade" => '\u{2122}',
    "hellip" => '\u{2026}',
    "bull" => '\u{2022}',
    "middot" => '\u{00B7}',
    "laquo" => '\u{00AB}',
    "raquo" => '\u{00BB}',
    "frac12" => '\u{00BD}',
    "frac14" => '\u{00BC}',
    "frac34" => '\u{00BE}',
    "deg" => '\u{00B0}',
    "micro" => '\u{00B5}',
    "para" => '\u{00B6}',
    "sect" => '\u{00A7}',
    "uml" => '\u{00A8}',
    "agrave" => '\u{00E0}',
    "egrave" => '\u{00E8}',
    "igrave" => '\u{00EC}',
    "ograve" => '\u{00F2}',
    "ugrave" => '\u{00F9}',
    "aacute" => '\u{00E1}',
    "iacute" => '\u{00ED}',
    "oacute" => '\u{00F3}',
    "uacute" => '\u{00FA}',
    "acirc" => '\u{00E2}',
    "ecirc" => '\u{00EA}',
    "icirc" => '\u{00EE}',
    "ocirc" => '\u{00F4}',
    "ucirc" => '\u{00FB}',
    "atilde" => '\u{00E3}',
    "ntilde" => '\u{00F1}',
    "otilde" => '\u{00F5}',
    "auml" => '\u{00E4}',
    "euml" => '\u{00EB}',
    "iuml" => '\u{00EF}',
    "ouml" => '\u{00F6}',
    "uuml" => '\u{00FC}',
    "ccedil" => '\u{00E7}',
    "szlig" => '\u{00DF}',
};

fn decode_entity(entity: &str) -> Option<char> {
    if let Some(num_str) = entity.strip_prefix('#') {
        let (radix, digits) = match num_str
            .strip_prefix('x')
            .or_else(|| num_str.strip_prefix('X'))
        {
            Some(hex) => (16, hex),
            None => (10, num_str),
        };
        return u32::from_str_radix(digits, radix)
            .ok()
            .and_then(char::from_u32);
    }
    NAMED_ENTITIES.get(entity).copied()
}

/// Extract sections from an HTML document by converting to markdown first.
pub fn extract_html_sections(file_path: &str, source: &str) -> Vec<Section> {
    let markdown = convert_html_to_markdown(source);
    extract_markdown_sections(file_path, &markdown)
}
