//! Identifier-boundary text matching shared across analysis + engine.
//! A single byte-level implementation that previously existed in 3 copies
//! (`codesight-analysis::graph::calls::word_match` and two wrappers in
//! `codesight-engine`) — consolidated here per AGENTS.md §L0.75.
//!
//! ASCII-only today: a "word character" is `[A-Za-z0-9_]`, matching the
//! identifier shape of every language we currently index. A future
//! Unicode-aware variant can live alongside without disrupting callers.

/// Whole-word match: returns `true` when `body` contains `name` with
/// non-word characters on both sides (or a string boundary).
#[must_use]
pub fn word_match(body: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let name_bytes = name.as_bytes();
    let body_bytes = body.as_bytes();
    let nlen = name_bytes.len();

    let mut start = 0;
    while start + nlen <= body_bytes.len() {
        if let Some(pos) = body[start..].find(name) {
            let abs_pos = start + pos;
            let before_ok = abs_pos == 0 || !is_word_char(body_bytes[abs_pos - 1]);
            let after_ok =
                abs_pos + nlen >= body_bytes.len() || !is_word_char(body_bytes[abs_pos + nlen]);
            if before_ok && after_ok {
                return true;
            }
            start = abs_pos + 1;
        } else {
            break;
        }
    }
    false
}

/// Case-insensitive whole-word match. Lowercases both sides before
/// delegating to [`word_match`].
#[must_use]
pub fn ci_word_match(body: &str, name: &str) -> bool {
    let body_lc = body.to_lowercase();
    let name_lc = name.to_lowercase();
    word_match(&body_lc, &name_lc)
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Yield every maximal run of `[A-Za-z0-9_]` in `src` — every
/// identifier-shaped token. Matches the [`word_match`] definition of
/// "word character" so downstream edge semantics stay aligned across the
/// call graph, references lookup, and dead-code reachability passes.
pub fn iter_identifiers(src: &str) -> impl Iterator<Item = &str> {
    src.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
}

/// 1-based line and column of the first whole-word (word-boundary)
/// occurrence of `identifier` in `source`, case-insensitive. `None` when
/// the identifier doesn't appear as a full word.
#[must_use]
pub fn locate_identifier_ci(source: &str, identifier: &str) -> Option<(u32, u32)> {
    if identifier.is_empty() {
        return None;
    }
    let id_lower = identifier.to_ascii_lowercase();
    let id_len = id_lower.len();
    for (line_idx, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        let line_lower = line.to_ascii_lowercase();
        let lower_bytes = line_lower.as_bytes();
        let mut search_from = 0usize;
        while let Some(pos) = line_lower[search_from..].find(&id_lower) {
            let abs = search_from + pos;
            let before_ok = abs == 0 || !is_word_char(bytes[abs - 1]);
            let end = abs + id_len;
            let after_ok = end >= lower_bytes.len() || !is_word_char(bytes[end]);
            if before_ok && after_ok {
                return Some((line_idx as u32 + 1, abs as u32 + 1));
            }
            search_from = abs + id_len;
        }
    }
    None
}

/// Drop everything between `"` pairs on a single line so substring probes
/// like "is `//` before me?" don't false-positive inside string literals.
#[must_use]
pub fn strip_strings_before(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    for ch in s.chars() {
        match ch {
            '"' => in_str = !in_str,
            _ if !in_str => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Coarse classifier for a whole-identifier hit on `line_text` at byte
/// column `ident_col` (0-based) and byte length `ident_len`. Returns one
/// of `"comment_or_string" | "use_stmt" | "call_site" | "type_ref" |
/// "other"`. Language-agnostic heuristics good enough for rename-safety
/// gating and `find_references` classification.
#[must_use]
pub fn classify_body_match(line_text: &str, ident_col: usize, ident_len: usize) -> &'static str {
    let trimmed = line_text.trim_start();
    let leading_ws = line_text.len() - trimmed.len();
    let col = ident_col.min(line_text.len());
    let end = (col + ident_len).min(line_text.len());
    let before_raw = &line_text[..col];
    let after_raw = line_text.get(end..).unwrap_or("");

    let before_no_strings = strip_strings_before(before_raw);
    if before_no_strings.contains("//")
        || before_no_strings.contains("/*")
        || before_no_strings.trim_start().starts_with('#')
    {
        return "comment_or_string";
    }
    let quote_count = before_raw.bytes().filter(|b| *b == b'"').count();
    if quote_count % 2 == 1 {
        return "comment_or_string";
    }

    let head: &str = &line_text[leading_ws..];
    for kw in [
        "use ",
        "import ",
        "from ",
        "require ",
        "require(",
        "pub use ",
        "#include ",
        "#include\"",
        "#include<",
    ] {
        if head.starts_with(kw) {
            return "use_stmt";
        }
    }

    let next = after_raw.trim_start_matches(|c: char| c == ':' || c.is_ascii_whitespace());
    if next.starts_with('(') {
        return "call_site";
    }
    if after_raw.trim_start().starts_with("::<") {
        return "call_site";
    }

    let before_trim = before_raw.trim_end();
    if before_trim.ends_with(':')
        || before_trim.ends_with("->")
        || before_trim.ends_with('<')
        || before_trim.ends_with("impl")
        || before_trim.ends_with(" for")
        || before_trim.ends_with("dyn")
    {
        return "type_ref";
    }

    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rejects_comment_and_string_hits() {
        assert_eq!(
            classify_body_match("    // rename the record field", 16, 6),
            "comment_or_string"
        );
        assert_eq!(
            classify_body_match("let s = \"record\";", 9, 6),
            "comment_or_string"
        );
        assert_eq!(
            classify_body_match("# update record here", 9, 6),
            "comment_or_string"
        );
    }

    #[test]
    fn classify_call_site_and_use_stmt() {
        assert_eq!(classify_body_match("ledger.record(x)", 7, 6), "call_site");
        assert_eq!(classify_body_match("use crate::record;", 11, 6), "use_stmt");
    }

    #[test]
    fn locate_identifier_ci_requires_word_boundary() {
        assert_eq!(locate_identifier_ci("let foo = 1;", "foo"), Some((1, 5)));
        assert!(locate_identifier_ci("let foobar = 1;", "foo").is_none());
    }
}
