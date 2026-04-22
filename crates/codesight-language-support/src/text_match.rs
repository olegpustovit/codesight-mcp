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
