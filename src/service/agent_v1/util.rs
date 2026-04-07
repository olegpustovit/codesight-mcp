//! Shared helpers for agent-v1 handlers.

use std::collections::BTreeMap;

use regex::Regex;
use serde_json::{Value, json};

use crate::config::BYTES_PER_TOKEN;
use crate::model::SearchSymbol;

/// Count parameters in a function/method signature string.
///
/// Splits on commas at bracket-depth 0, ignoring commas inside `<>`, `[]`,
/// `()` etc.  Special-cases C `(void)` as zero parameters.
pub(in crate::service) fn count_params(signature: &str) -> usize {
    // Find the parameter list between the outermost parentheses.
    let Some(open) = signature.find('(') else {
        return 0;
    };
    // Find matching close paren (respecting nesting).
    let after_open = open + 1;
    let bytes = signature.as_bytes();
    let mut depth: i32 = 1;
    let mut close = signature.len();
    for i in after_open..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = signature[after_open..close].trim();
    if inner.is_empty() {
        return 0;
    }
    // C convention: `(void)` means zero params — but `(void *…)` is one param.
    if inner == "void" {
        return 0;
    }
    // Split on commas at bracket-depth 0.
    let mut params = 1usize;
    let mut bracket_depth: i32 = 0;
    for ch in inner.chars() {
        match ch {
            '<' | '[' | '(' => bracket_depth += 1,
            '>' | ']' | ')' => bracket_depth -= 1,
            ',' if bracket_depth == 0 => params += 1,
            _ => {}
        }
    }
    params
}

/// Simple glob matching: supports `*` (matches any non-`/` chars) and `**` is not special.
pub(in crate::service) fn glob_match(pattern: &str, path: &str) -> bool {
    let re_str = pattern.replace('.', r"\.").replace('*', "[^/]*");
    regex::Regex::new(&format!("(?:^|/){re_str}$"))
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

/// Maximum nesting depth in a source body.
///
/// For brace-delimited languages this counts the deepest `{` nesting.
/// For indentation-based languages (Python, etc.) this measures indentation
/// depth relative to the first line's indentation.
pub(in crate::service) fn max_nesting_depth(body: &str) -> usize {
    // Try brace-based first.
    let mut max_depth: usize = 0;
    let mut depth: usize = 0;
    for ch in body.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth > max_depth {
                    max_depth = depth;
                }
            }
            '}' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    if max_depth > 0 {
        return max_depth;
    }

    // Fall back to indentation-based nesting (Python, etc.).
    // Skip the first non-empty line (function signature) and use the second
    // non-empty line's indentation as baseline. This way a flat function body
    // (signature + uniformly-indented statements) yields nesting depth 0.
    let mut base_indent: Option<usize> = None;
    let mut max_indent_depth: usize = 0;
    let mut skipped_signature = false;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !skipped_signature {
            skipped_signature = true;
            continue; // skip first non-empty line (function/method signature)
        }
        let indent = line.len() - line.trim_start().len();
        let base = *base_indent.get_or_insert(indent);
        let relative = indent.saturating_sub(base);
        // Assume 4-space or 2-space indent; pick the larger unit that divides evenly,
        // defaulting to 4.
        let unit = 4;
        let level = relative / unit;
        if level > max_indent_depth {
            max_indent_depth = level;
        }
    }
    max_indent_depth
}
pub(in crate::service) fn search_hit_symbol(source_id: &str, s: &SearchSymbol) -> Value {
    let mut hit = serde_json::Map::new();
    hit.insert(
        "ref".to_string(),
        json!({
            "id": s.id,
            "stable_id": s.id,
            "source_id": source_id,
            "domain": "code",
            "kind": "symbol",
            "path": s.file,
            "title": s.name,
            "summary": s.summary,
        }),
    );
    hit.insert("snippet".to_string(), json!(s.signature));
    hit.insert("score".to_string(), json!(s.score));
    hit.insert("line".to_string(), json!(s.line));
    if let Some(ref m) = s.match_type {
        hit.insert("match_type".to_string(), json!(m));
    }
    if let Some(v) = s.fuzzy_similarity {
        hit.insert("fuzzy_similarity".to_string(), json!(v));
    }
    if let Some(v) = s.edit_distance {
        hit.insert("edit_distance".to_string(), json!(v));
    }
    Value::Object(hit)
}

/// Rough token count for ranked `total_tokens` / budget checks (`BYTES_PER_TOKEN` bytes ≈ one token).
pub(in crate::service) fn token_estimate_for_context(source: &str) -> u32 {
    if source.is_empty() {
        return 0;
    }
    let b = source.len() as u64;
    let per = BYTES_PER_TOKEN as u64;
    ((b.saturating_add(per.saturating_sub(1))) / per).clamp(1, u32::MAX as u64) as u32
}

/// Greedy pack by estimated JSON serialization bytes (`BYTES_PER_TOKEN` per budget unit).
pub(in crate::service) fn pack_catalog_items_to_token_budget(
    items: Vec<Value>,
    max_tokens: u32,
) -> (Vec<Value>, bool) {
    if max_tokens == 0 {
        return (Vec::new(), !items.is_empty());
    }
    let total = items.len();
    let budget_bytes = (max_tokens as u64).saturating_mul(BYTES_PER_TOKEN);
    let mut used: u64 = 0;
    let mut kept = Vec::with_capacity(total);
    for it in items {
        let cost = serde_json::to_string(&it)
            .map(|s| s.len() as u64)
            .unwrap_or(64);
        let cost = cost.max(1);
        if used.saturating_add(cost) > budget_bytes {
            break;
        }
        used += cost;
        kept.push(it);
    }
    let truncated = kept.len() < total;
    (kept, truncated)
}

pub(in crate::service) fn err_item(code: &str, message: &str) -> Value {
    json!({
        "error": { "code": code, "message": message }
    })
}

fn append_section_neighbor(
    out: &mut Vec<Value>,
    row: &Value,
    source_id: &str,
    skip_id: &str,
) -> bool {
    let Some(nid) = row.get("id").and_then(|x| x.as_str()) else {
        return false;
    };
    if nid.is_empty() || nid == skip_id {
        return false;
    }
    out.push(json!({
        "ref": {
            "kind": "section",
            "stable_id": nid,
            "source_id": source_id,
        },
        "heading": row.get("heading"),
        "line": row.get("line"),
        "summary": row.get("summary"),
    }));
    true
}

pub(in crate::service) fn doc_format_to_type_label(format: &str) -> String {
    match format {
        "markdown" => ".md".to_string(),
        "text" => ".txt".to_string(),
        "rst" => ".rst".to_string(),
        "asciidoc" => ".adoc".to_string(),
        other => format!(".{other}"),
    }
}

pub(in crate::service) fn truncate_content_to_token_budget(
    content: &str,
    max_tokens: u32,
) -> (String, bool) {
    let max_bytes = (max_tokens as u64).saturating_mul(crate::config::BYTES_PER_TOKEN) as usize;
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }
    let mut end = max_bytes.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (content[..end].to_string(), true)
}

fn heading_path_ancestor_prefixes(heading_path: &str) -> Vec<String> {
    let parts: Vec<&str> = heading_path.split(" > ").collect();
    if parts.len() <= 1 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..parts.len() - 1 {
        out.push(parts[..=i].join(" > "));
    }
    out
}

pub(in crate::service) fn heading_path_ancestors(all: &[Value], heading_path: &str) -> Vec<Value> {
    let mut acc = Vec::new();
    for pref in heading_path_ancestor_prefixes(heading_path) {
        if let Some(row) = all
            .iter()
            .find(|r| r.get("heading_path").and_then(|x| x.as_str()) == Some(pref.as_str()))
        {
            acc.push(json!({
                "title": row.get("heading"),
                "id": row.get("id"),
                "level": row.get("level"),
                "line": row.get("line"),
            }));
        }
    }
    acc
}

pub(in crate::service) fn direct_child_sections(
    all: &[Value],
    parent_heading_path: &str,
) -> Vec<Value> {
    let prefix = format!("{} > ", parent_heading_path);
    all.iter()
        .filter_map(|row| {
            let hp = row.get("heading_path").and_then(|x| x.as_str())?;
            if !hp.starts_with(&prefix) {
                return None;
            }
            let rest = &hp[prefix.len()..];
            if rest.contains(" > ") {
                return None;
            }
            Some(json!({
                "title": row.get("heading"),
                "id": row.get("id"),
                "level": row.get("level"),
                "line": row.get("line"),
                "summary": row.get("summary"),
            }))
        })
        .collect()
}

pub(in crate::service) fn outline_neighbors(
    all: &[Value],
    sec_id: &str,
    max_neighbors: usize,
    source_id: &str,
) -> Vec<Value> {
    let idx = all
        .iter()
        .position(|row| row.get("id").and_then(|x| x.as_str()) == Some(sec_id));
    let mut out: Vec<Value> = Vec::new();
    let Some(idx) = idx else {
        return out;
    };
    let mut offset = 1usize;
    while out.len() < max_neighbors {
        let mut progressed = false;
        if idx >= offset {
            if let Some(row) = all.get(idx - offset) {
                if append_section_neighbor(&mut out, row, source_id, sec_id) {
                    progressed = true;
                }
            }
        }
        if out.len() >= max_neighbors {
            break;
        }
        if idx + offset < all.len() {
            if let Some(row) = all.get(idx + offset) {
                if append_section_neighbor(&mut out, row, source_id, sec_id) {
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
        offset += 1;
    }
    out
}
