//! Shared parsing helpers for `ComplexityModel::measured_type_density`.
//!
//! The original scattered tables in `codesight-analysis::agent_readiness`
//! shared a handful of small parsers (`TopLevelScanner`, `split_top_level`,
//! `find_matching_delimiter`, `type_token_score`). Each language calls into
//! one of four measurement strategies below.

use std::str::CharIndices;

/// Exact-match tokens that collapse agent-readable types to "any".
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

/// PHP-style `Type $name` parameter syntax with `: ReturnType` suffix.
pub fn measure_php_signature(source: &str) -> Option<f64> {
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let params = &source[open + 1..close];
    let suffix = &source[close + 1..];

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() {
            continue;
        }
        total_slots += 1;
        let tokens: Vec<&str> = param.split_whitespace().collect();
        if let Some(var_pos) = tokens.iter().position(|token| token.starts_with('$'))
            && var_pos > 0
        {
            score_sum += type_token_score(&tokens[..var_pos].join(" "), "php");
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, "php") {
        score_sum += type_token_score(return_ty, "php");
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

/// Go-style `func (recv) name(params) ret`.
pub fn measure_go_signature(source: &str) -> Option<f64> {
    let func_idx = source.find("func")?;
    let mut scan = source[func_idx + 4..].trim_start();
    if scan.starts_with('(') {
        let open = source.len() - scan.len();
        let close = find_matching_delimiter(source, open, '(', ')')?;
        scan = source[close + 1..].trim_start();
    }

    let name_paren_offset = scan.find('(')?;
    let params_open = source.len() - scan.len() + name_paren_offset;
    let params_close = find_matching_delimiter(source, params_open, '(', ')')?;
    let params = &source[params_open + 1..params_close];
    let suffix = source[params_close + 1..].trim_start();

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    let mut pending_names = 0usize;

    for raw_part in split_top_level(params, ',') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        if !part.contains(' ') && !part.starts_with("...") {
            pending_names += 1;
            continue;
        }

        let Some(last_space) = part.rfind(' ') else {
            total_slots += 1 + pending_names;
            pending_names = 0;
            continue;
        };
        let names = part[..last_space]
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .count()
            + pending_names;
        let ty = part[last_space + 1..].trim();
        total_slots += names.max(1);
        score_sum += type_token_score(ty, "go") * names.max(1) as f64;
        pending_names = 0;
    }
    if pending_names > 0 {
        total_slots += pending_names;
    }

    total_slots += 1;
    if suffix.starts_with('{') {
        score_sum += 1.0;
    } else {
        let return_ty = take_until_top_level_char(suffix, &['{']).trim();
        if return_ty.is_empty() {
            score_sum += 1.0;
        } else {
            score_sum += type_token_score(return_ty, "go");
        }
    }

    Some(score_sum / total_slots as f64)
}

pub fn is_python_receiver(param: &str) -> bool {
    let name = param.split(':').next().unwrap_or(param).trim();
    matches!(name, "self" | "cls" | "*self" | "*cls")
}

pub fn is_rust_receiver(param: &str) -> bool {
    let normalized = param.replace(' ', "");
    matches!(
        normalized.as_str(),
        "self" | "&self" | "&mutself" | "mutself" | "self:Self" | "self:Box<Self>"
    ) || normalized.starts_with("self:")
}

fn extract_colon_param_type(param: &str) -> Option<&str> {
    let colon = find_top_level_colon(param)?;
    let annotation = param[colon + 1..].trim();
    let annotation = take_until_top_level_char(annotation, &['=']).trim();
    (!annotation.is_empty()).then_some(annotation)
}

fn extract_return_type_after_params<'a>(suffix: &'a str, language: &str) -> Option<&'a str> {
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

fn type_token_score(token: &str, language: &str) -> f64 {
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

struct TopLevelScanner<'a> {
    inner: CharIndices<'a>,
    paren: usize,
    bracket: usize,
    brace: usize,
    angle: usize,
    in_string: Option<char>,
    escape: bool,
}

impl<'a> TopLevelScanner<'a> {
    fn new(text: &'a str) -> Self {
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

    fn at_top_level(&self) -> bool {
        self.in_string.is_none()
            && self.paren == 0
            && self.bracket == 0
            && self.brace == 0
            && self.angle == 0
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

fn find_matching_delimiter(
    text: &str,
    open_index: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    let mut scanner = TopLevelScanner::new(&text[open_index..]);
    while let Some((idx, ch)) = scanner.next() {
        if scanner.in_string.is_some() {
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

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
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

fn take_until_top_level_char<'a>(text: &'a str, stop_chars: &[char]) -> &'a str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_unit_return_counts_as_typed() {
        let source =
            "fn run(&self, value: i32) {\n    if value > 1 {\n        println!(\"{value}\");\n    }\n}";
        let score =
            measure_colon_after_name(source, "rust", true, is_rust_receiver).unwrap();
        assert!((score - 1.0).abs() < 0.001, "got {score}");
    }

    #[test]
    fn go_grouped_parameters_and_void_return_are_typed() {
        let source = "func foo(x, y int) {\n    fmt.Println(x + y)\n}";
        let score = measure_go_signature(source).unwrap();
        assert!((score - 1.0).abs() < 0.001, "got {score}");
    }
}
