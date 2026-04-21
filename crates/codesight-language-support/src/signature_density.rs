//! Signature-shape parsing algorithms for `Language::measured_type_density`.
//!
//! This module contains ALGORITHMS, not languages:
//!   * `measure_colon_after_name` — generic parameterized parser for
//!     `name(param: Type, …) -> Ret` shapes (Rust, Python, TS, Kotlin, Swift).
//!   * `measure_type_before_name` — generic parser for `RetType name(ParamType n, …)`
//!     shapes (C, C++, Java, C#, Dart).
//!   * A reusable `TopLevelScanner` and helpers (delimiter matching, top-level
//!     splits, type-token weak-typing detection) that each language's own
//!     signature parser can compose with.
//!
//! Language-specific signature parsers (Go's `func (recv) name(params) ret`,
//! PHP's `Type $name`, receiver detectors) live in each language's
//! `languages/<lang>/mod.rs`.

use std::str::CharIndices;

/// Exact-match tokens that collapse to "any"-typed under any language.
const WEAK_TYPE_EXACT: &[&str] = &[
    "any",
    "typing.any",
    "object",
    "dynamic",
    "mixed",
    "interface{}",
    "void*",
    "auto",
    "t.untyped",
    "term()",
    "anyref",
    "anyobject",
    "dynany",
    "box<dynany>",
    "{*}",
];

/// Substrings that force a zero score regardless of language.
const WEAK_TYPE_CONTAINS: &[&str] = &["<any>", "typing.any", "dynany"];

/// Score a type token: 1.0 for real types, 0.0 for weak/any variants,
/// 0.3 for TypeScript/TSX `unknown`.
pub fn type_token_score(token: &str, language: &str) -> f64 {
    let normalized = normalize_type_token(token);
    if normalized.is_empty() {
        return 0.0;
    }
    if normalized == "unknown" && matches!(language, "typescript" | "tsx") {
        return 0.3;
    }
    if WEAK_TYPE_EXACT.contains(&normalized.as_str())
        || WEAK_TYPE_CONTAINS.iter().any(|s| normalized.contains(s))
    {
        return 0.0;
    }
    1.0
}

fn normalize_type_token(token: &str) -> String {
    token
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\'')
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

/// Measure colon-after-name family (Rust, Python, TS, JS, Kotlin, Swift,
/// Scala). `implicit_unit_return` is Rust's `()` default; `is_receiver`
/// filters `self` / `cls`.
pub fn measure_colon_after_name<F>(
    source: &str,
    language: &str,
    implicit_unit_return: bool,
    is_receiver: F,
) -> Option<f64>
where
    F: Fn(&str) -> bool,
{
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let params = &source[open + 1..close];
    let suffix = &source[close + 1..];

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() || param == "/" || param == "*" || is_receiver(param) {
            continue;
        }
        total_slots += 1;
        if let Some(ty) = extract_colon_param_type(param) {
            score_sum += type_token_score(ty, language);
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, language) {
        score_sum += type_token_score(return_ty, language);
    } else if implicit_unit_return {
        score_sum += 1.0;
    }

    Some(score_sum / total_slots as f64)
}

/// C/C++/Java/C#/Dart family: `RetType name(ParamType name, ...)`.
pub fn measure_type_before_name(source: &str, language: &str) -> Option<f64> {
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let before_open = source[..open].trim();
    let params = &source[open + 1..close];

    let return_type = infer_prefixed_return_type(before_open)?;
    let mut total_slots = 1usize;
    let mut score_sum = type_token_score(&return_type, language);

    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() {
            continue;
        }
        total_slots += 1;
        if let Some(ty) = extract_prefixed_param_type(param) {
            score_sum += type_token_score(&ty, language);
        }
    }

    Some(score_sum / total_slots as f64)
}

fn extract_colon_param_type(param: &str) -> Option<&str> {
    let colon = find_top_level_colon(param)?;
    let annotation = param[colon + 1..].trim();
    let annotation = take_until_top_level_char(annotation, &['=']).trim();
    (!annotation.is_empty()).then_some(annotation)
}

/// Return-type extractor: `: Type` (TS, Kotlin, Scala, PHP) or `-> Type`
/// (Rust, Python, Go). Exposed for per-language callers that need it.
pub fn extract_return_type_after_params<'a>(suffix: &'a str, language: &str) -> Option<&'a str> {
    let trimmed = suffix.trim_start();
    if matches!(language, "typescript" | "tsx" | "kotlin" | "scala" | "php")
        && trimmed.starts_with(':')
    {
        let ty = take_until_top_level_char(trimmed[1..].trim_start(), &['{', '=', ';'])
            .trim_end_matches(':')
            .trim();
        return (!ty.is_empty()).then_some(ty);
    }
    if let Some(idx) = find_top_level_arrow(trimmed) {
        let ty =
            take_until_top_level_char(trimmed[idx + 2..].trim_start(), &['{', ':', '=']).trim();
        return (!ty.is_empty()).then_some(ty);
    }
    None
}

fn infer_prefixed_return_type(before_open: &str) -> Option<String> {
    let trimmed = before_open.trim();
    let mut tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    tokens.pop()?;
    while matches!(
        tokens.first().copied(),
        Some(
            "pub"
                | "public"
                | "private"
                | "protected"
                | "internal"
                | "static"
                | "abstract"
                | "async"
                | "virtual"
                | "override"
                | "sealed"
                | "open"
                | "final"
                | "synchronized"
                | "inline"
                | "constexpr"
                | "extern"
                | "unsafe"
        )
    ) {
        tokens.remove(0);
    }
    (!tokens.is_empty()).then(|| tokens.join(" "))
}

fn extract_prefixed_param_type(param: &str) -> Option<String> {
    let trimmed = param.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        return None;
    }
    let mut start = 0usize;
    while matches!(
        tokens.get(start).copied(),
        Some("final" | "readonly" | "ref" | "out" | "in" | "params" | "this")
    ) {
        start += 1;
    }
    if tokens.len() <= start + 1 {
        return None;
    }
    let joined = tokens[start..tokens.len() - 1].join(" ");
    (!joined.is_empty()).then_some(joined)
}

// ── Reusable scanners (exposed for per-language signature parsers) ──────────

pub struct TopLevelScanner<'a> {
    inner: CharIndices<'a>,
    paren: usize,
    bracket: usize,
    brace: usize,
    angle: usize,
    in_string: Option<char>,
    escape: bool,
}

impl<'a> TopLevelScanner<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            inner: text.char_indices(),
            paren: 0,
            bracket: 0,
            brace: 0,
            angle: 0,
            in_string: None,
            escape: false,
        }
    }

    pub fn at_top_level(&self) -> bool {
        self.in_string.is_none()
            && self.paren == 0
            && self.bracket == 0
            && self.brace == 0
            && self.angle == 0
    }

    pub fn in_string(&self) -> Option<char> {
        self.in_string
    }
}

impl Iterator for TopLevelScanner<'_> {
    type Item = (usize, char);

    fn next(&mut self) -> Option<(usize, char)> {
        let (idx, ch) = self.inner.next()?;

        if let Some(quote) = self.in_string {
            if self.escape {
                self.escape = false;
            } else if ch == '\\' {
                self.escape = true;
            } else if ch == quote {
                self.in_string = None;
            }
            return Some((idx, ch));
        }

        match ch {
            '"' | '`' => self.in_string = Some(ch),
            '(' => self.paren += 1,
            ')' => self.paren = self.paren.saturating_sub(1),
            '[' => self.bracket += 1,
            ']' => self.bracket = self.bracket.saturating_sub(1),
            '{' => self.brace += 1,
            '}' => self.brace = self.brace.saturating_sub(1),
            '<' => self.angle += 1,
            '>' => self.angle = self.angle.saturating_sub(1),
            _ => {}
        }
        Some((idx, ch))
    }
}

pub fn find_matching_delimiter(
    text: &str,
    open_index: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    let mut scanner = TopLevelScanner::new(&text[open_index..]);
    while let Some((idx, ch)) = scanner.next() {
        if scanner.in_string().is_some() {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(open_index + idx);
            }
        }
    }
    None
}

pub fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == delimiter && scanner.at_top_level() {
            parts.push(&text[start..idx]);
            start = idx + ch.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

pub fn take_until_top_level_char<'a>(text: &'a str, stop_chars: &[char]) -> &'a str {
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if scanner.at_top_level() && stop_chars.contains(&ch) {
            return &text[..idx];
        }
    }
    text
}

fn find_top_level_colon(text: &str) -> Option<usize> {
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == ':' && scanner.at_top_level() {
            return Some(idx);
        }
    }
    None
}

fn find_top_level_arrow(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == '-' && idx + 1 < bytes.len() && bytes[idx + 1] == b'>' && scanner.at_top_level() {
            return Some(idx);
        }
    }
    None
}
