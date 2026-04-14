//! Shared helpers for agent-v1 handlers.

use serde_json::{Value, json};

use crate::config::BYTES_PER_TOKEN;
use crate::storage::{SearchSection, SearchSymbol};

/// Round a ranker score to 3 decimal places for JSON serialization so f64
/// round-trip artifacts (e.g. `0.5299999999999999`) don't leak into the
/// response envelope. Internal comparisons stay exact — truncation happens
/// only at the JSON boundary.
pub(in crate::service) fn round_score(score: f64) -> f64 {
    if score.is_finite() {
        (score * 1000.0).round() / 1000.0
    } else {
        score
    }
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
    hit.insert("score".to_string(), json!(round_score(s.score)));
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

pub(in crate::service) fn search_hit_section(source_id: &str, s: &SearchSection) -> Value {
    json!({
        "ref": {
            "id": s.id,
            "stable_id": s.id,
            "source_id": source_id,
            "domain": "docs",
            "kind": "section",
            "path": s.file,
            "title": s.heading,
            "summary": s.summary,
        },
        "snippet": s.heading_path,
        "score": round_score(s.score),
        "line": s.line,
    })
}

/// Rough token count for ranked `total_tokens` / budget checks (`BYTES_PER_TOKEN` bytes ≈ one token).
pub(in crate::service) fn token_estimate_for_context(source: &str) -> u32 {
    if source.is_empty() {
        return 0;
    }
    let b = source.len() as u64;
    let per = BYTES_PER_TOKEN;
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

/// Resolve an agent-v1 symbol `PrefRef` (from `read_content`, `context_bundle`,
/// `evaluate_impact`, etc.) to its indexed stable id.
///
/// Order: explicit `stable_id` → explicit `id` → `path` + `start_line` lookup
/// against the symbol index. Returns `None` when none of the above produce a
/// usable id (callers turn that into a `MISSING_REF` error).
pub(in crate::service) fn resolve_symbol_id(
    store: &crate::storage::IndexStore,
    stable_id: Option<&str>,
    id: Option<&str>,
    path: Option<&str>,
    start_line: Option<u32>,
) -> anyhow::Result<Option<String>> {
    if let Some(s) = stable_id.filter(|s| !s.is_empty()) {
        return Ok(Some(s.to_string()));
    }
    if let Some(s) = id.filter(|s| !s.is_empty()) {
        return Ok(Some(s.to_string()));
    }
    if let (Some(p), Some(line)) = (path, start_line)
        && line > 0
    {
        return store.symbol_id_at_location(p, line);
    }
    Ok(None)
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
        if idx >= offset
            && let Some(row) = all.get(idx - offset)
            && append_section_neighbor(&mut out, row, source_id, sec_id)
        {
            progressed = true;
        }
        if out.len() >= max_neighbors {
            break;
        }
        if idx + offset < all.len()
            && let Some(row) = all.get(idx + offset)
            && append_section_neighbor(&mut out, row, source_id, sec_id)
        {
            progressed = true;
        }
        if !progressed {
            break;
        }
        offset += 1;
    }
    out
}
