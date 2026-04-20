//! Shared helpers for agent-v1 handlers.

use serde_json::{Value, json};

use crate::config::BYTES_PER_TOKEN;
use codesight_storage::storage::{SearchSymbol, doc_index::SearchSection};

/// Test whether a repo-relative file path passes the combined
/// scope / exclude / file_pattern filter used by `inspect_graph` and
/// `find_references` (FEEDBACK.md §6b).
///
/// - `scope`: substring inclusion (path must contain it).
/// - `exclude`: substrings to skip (any match rejects).
/// - `file_pattern`: `*` / `**` / `?` glob — matches the full path.
///
/// All `None` means unfiltered (accept).
pub(in crate::service) fn path_matches_filter(
    path: &str,
    scope: Option<&str>,
    exclude: Option<&[String]>,
    file_pattern: Option<&str>,
) -> bool {
    if let Some(scope) = scope
        && !scope.is_empty()
        && !path.contains(scope)
    {
        return false;
    }
    if let Some(excludes) = exclude {
        for ex in excludes {
            if !ex.is_empty() && path.contains(ex) {
                return false;
            }
        }
    }
    if let Some(pat) = file_pattern
        && !pat.is_empty()
        && !glob_match_path(pat, path)
    {
        return false;
    }
    true
}

/// Minimal glob matcher: `**` matches any sequence (including `/`), `*`
/// matches any non-`/` sequence, `?` matches a single char. Good enough for
/// "match these repo-relative paths" without pulling in a full globset.
fn glob_match_path(pattern: &str, path: &str) -> bool {
    // Convert to a regex: `**` → `.*`, `*` → `[^/]*`, `?` → `.`, escape
    // other special chars. Anchored at both ends.
    let mut regex = String::with_capacity(pattern.len() + 4);
    regex.push('^');
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'*' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    regex.push_str(".*");
                    i += 2;
                } else {
                    regex.push_str("[^/]*");
                    i += 1;
                }
            }
            b'?' => {
                regex.push('.');
                i += 1;
            }
            b'.' | b'+' | b'(' | b')' | b'|' | b'^' | b'$' | b'{' | b'}' | b'[' | b']'
            | b'\\' => {
                regex.push('\\');
                regex.push(bytes[i] as char);
                i += 1;
            }
            other => {
                regex.push(other as char);
                i += 1;
            }
        }
    }
    regex.push('$');
    regex::Regex::new(&regex)
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

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

pub(in crate::service) fn search_hit_symbol(
    _source_id: &str,
    s: &SearchSymbol,
    body: Option<&str>,
) -> Value {
    // `ref` carries only the fields that can't be reconstructed from the
    // stable id's `path::name#kind` format. `stable_id`, `source_id`,
    // `domain`, `kind`, and `summary` are redundant (see PLAN2 §3.1).
    let mut hit = serde_json::Map::new();
    hit.insert(
        "ref".to_string(),
        json!({
            "id": s.id,
            "path": s.file,
            "title": s.name,
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
    if let Some(body) = body {
        hit.insert("body".to_string(), json!(body));
    }
    Value::Object(hit)
}

pub(in crate::service) fn search_hit_section(
    _source_id: &str,
    s: &SearchSection,
    body: Option<&str>,
) -> Value {
    // Ref carries only what the id doesn't — heading (title), file, line,
    // and heading_path as a snippet. `summary` duplicates heading_path;
    // `stable_id` / `source_id` / `domain` / `kind` are redundant.
    let mut hit = serde_json::Map::new();
    hit.insert(
        "ref".to_string(),
        json!({
            "id": s.id,
            "path": s.file,
            "title": s.heading,
        }),
    );
    hit.insert("snippet".to_string(), json!(s.heading_path));
    hit.insert("score".to_string(), json!(round_score(s.score)));
    hit.insert("line".to_string(), json!(s.line));
    if let Some(body) = body {
        hit.insert("body".to_string(), json!(body));
    }
    Value::Object(hit)
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
/// → `path` + `symbol_name` → unique bare `symbol_name`. Returns `None` when
/// nothing resolves (callers turn that into a `MISSING_REF` / ambiguity error).
pub(in crate::service) fn resolve_symbol_id(
    store: &codesight_storage::storage::IndexStore,
    stable_id: Option<&str>,
    id: Option<&str>,
    path: Option<&str>,
    start_line: Option<u32>,
    symbol_name: Option<&str>,
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
    if let Some(name) = symbol_name.filter(|s| !s.is_empty()) {
        if let Some(p) = path.filter(|s| !s.is_empty()) {
            return store.symbol_id_at_name(p, name);
        }
        return store.symbol_id_by_unique_name(name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use codesight_core::symbols::SymbolKind;

    fn sym(id: &str, name: &str, file: &str, line: u32) -> SearchSymbol {
        SearchSymbol {
            id: id.to_string(),
            kind: SymbolKind::Function,
            name: name.to_string(),
            file: file.to_string(),
            line,
            signature: format!("fn {name}()"),
            summary: String::new(),
            score: 0.7654321,
            match_type: Some("exact".to_string()),
            fuzzy_similarity: None,
            edit_distance: None,
        }
    }

    fn section_row(id: &str, heading: &str, heading_path: &str, line: u32, level: u32) -> Value {
        json!({
            "id": id,
            "heading": heading,
            "heading_path": heading_path,
            "line": line,
            "level": level,
            "summary": format!("summary of {heading}"),
        })
    }

    #[test]
    fn round_score_truncates_and_passes_through_nonfinite() {
        assert_eq!(round_score(0.5299999999999999), 0.53);
        assert_eq!(round_score(1.2345678), 1.235);
        assert!(round_score(f64::NAN).is_nan());
        assert_eq!(round_score(f64::INFINITY), f64::INFINITY);
    }

    #[test]
    fn search_hit_symbol_shapes_ref_and_rounds_score() {
        let s = sym("my_id", "do_thing", "src/lib.rs", 42);
        let hit = search_hit_symbol("src-1", &s, Some("fn body"));
        assert_eq!(hit["ref"]["id"], "my_id");
        assert_eq!(hit["ref"]["path"], "src/lib.rs");
        assert_eq!(hit["ref"]["title"], "do_thing");
        // Redundant envelope fields are dropped (PLAN2 §3.1).
        let r = hit["ref"].as_object().unwrap();
        assert!(r.get("stable_id").is_none());
        assert!(r.get("source_id").is_none());
        assert!(r.get("domain").is_none());
        assert!(r.get("kind").is_none());
        assert!(r.get("summary").is_none());
        assert_eq!(hit["score"], 0.765);
        assert_eq!(hit["line"], 42);
        assert_eq!(hit["match_type"], "exact");
        assert_eq!(hit["body"], "fn body");
    }

    #[test]
    fn search_hit_symbol_omits_optional_fields_when_absent() {
        let mut s = sym("id", "n", "f.rs", 1);
        s.match_type = None;
        let hit = search_hit_symbol("src", &s, None);
        assert!(hit.as_object().unwrap().get("match_type").is_none());
        assert!(hit.as_object().unwrap().get("body").is_none());
    }

    #[test]
    fn token_estimate_ceil_divides_and_clamps_empty() {
        assert_eq!(token_estimate_for_context(""), 0);
        assert_eq!(token_estimate_for_context("a"), 1);
        assert_eq!(token_estimate_for_context("abcd"), 1);
        assert_eq!(token_estimate_for_context("abcde"), 2);
        assert_eq!(
            token_estimate_for_context(&"x".repeat(4000)),
            (4000 / BYTES_PER_TOKEN) as u32
        );
    }

    #[test]
    fn pack_catalog_items_respects_zero_budget() {
        let items = vec![json!({"a": 1}), json!({"b": 2})];
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 0);
        assert!(kept.is_empty());
        assert!(truncated);
    }

    #[test]
    fn pack_catalog_items_zero_budget_zero_items_not_truncated() {
        let (kept, truncated) = pack_catalog_items_to_token_budget(Vec::new(), 0);
        assert!(kept.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn pack_catalog_items_stops_when_exceeding_budget() {
        let items = vec![json!({"k": "a".repeat(30)}), json!({"k": "b".repeat(30)})];
        // first item ~40 bytes serialized → 10 tokens; budget=10 tokens ≈ 40 bytes keeps one
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 10);
        assert_eq!(kept.len(), 1);
        assert!(truncated);
    }

    #[test]
    fn pack_catalog_items_keeps_all_when_budget_generous() {
        let items = vec![json!({"a": 1}), json!({"b": 2})];
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 1_000);
        assert_eq!(kept.len(), 2);
        assert!(!truncated);
    }

    #[test]
    fn truncate_content_honors_char_boundary() {
        let content = "αβγδ"; // each char is 2 bytes in UTF-8
        // 2 tokens * 4 bytes = 8 bytes cap → full content kept (8 bytes)
        let (out, truncated) = truncate_content_to_token_budget(content, 2);
        assert_eq!(out, content);
        assert!(!truncated);
    }

    #[test]
    fn truncate_content_drops_past_budget_and_aligns_boundary() {
        let content = "αβγδ"; // 8 bytes
        // 1 token * 4 bytes = 4 bytes cap; 4 bytes = 2 full chars → aligned
        let (out, truncated) = truncate_content_to_token_budget(content, 1);
        assert_eq!(out, "αβ");
        assert!(truncated);
    }

    #[test]
    fn truncate_content_rolls_back_to_valid_boundary() {
        // 5 bytes would land mid-char; ensure output is a valid prefix.
        let content = "αβγ"; // 6 bytes
        let per = BYTES_PER_TOKEN as u32; // 4
        let (out, truncated) = truncate_content_to_token_budget(content, per + 1);
        assert!(truncated || out == content);
        assert!(content.starts_with(&out));
    }

    #[test]
    fn doc_format_to_type_label_maps_known_and_fallbacks() {
        assert_eq!(doc_format_to_type_label("markdown"), ".md");
        assert_eq!(doc_format_to_type_label("text"), ".txt");
        assert_eq!(doc_format_to_type_label("rst"), ".rst");
        assert_eq!(doc_format_to_type_label("asciidoc"), ".adoc");
        assert_eq!(doc_format_to_type_label("org"), ".org");
    }

    #[test]
    fn err_item_wraps_code_and_message() {
        let v = err_item("BAD", "oops");
        assert_eq!(v["error"]["code"], "BAD");
        assert_eq!(v["error"]["message"], "oops");
    }

    #[test]
    fn heading_path_ancestors_returns_chain_excluding_self() {
        let all = vec![
            section_row("r", "Root", "Root", 1, 1),
            section_row("c", "Child", "Root > Child", 2, 2),
            section_row("g", "Grand", "Root > Child > Grand", 3, 3),
        ];
        let anc = heading_path_ancestors(&all, "Root > Child > Grand");
        assert_eq!(anc.len(), 2);
        assert_eq!(anc[0]["id"], "r");
        assert_eq!(anc[1]["id"], "c");
    }

    #[test]
    fn heading_path_ancestors_empty_for_top_level() {
        let all = vec![section_row("r", "Root", "Root", 1, 1)];
        assert!(heading_path_ancestors(&all, "Root").is_empty());
    }

    #[test]
    fn direct_child_sections_only_returns_depth_one_descendants() {
        let all = vec![
            section_row("r", "Root", "Root", 1, 1),
            section_row("c1", "Child1", "Root > Child1", 2, 2),
            section_row("c2", "Child2", "Root > Child2", 4, 2),
            section_row("g", "Grand", "Root > Child1 > Grand", 3, 3),
        ];
        let kids = direct_child_sections(&all, "Root");
        assert_eq!(kids.len(), 2);
        let ids: Vec<&str> = kids.iter().map(|k| k["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"c1"));
        assert!(ids.contains(&"c2"));
    }

    #[test]
    fn outline_neighbors_walks_symmetrically_and_skips_self() {
        let all = vec![
            section_row("a", "A", "A", 1, 1),
            section_row("b", "B", "A > B", 2, 2),
            section_row("c", "C", "A > C", 3, 2),
            section_row("d", "D", "A > D", 4, 2),
            section_row("e", "E", "A > E", 5, 2),
        ];
        let out = outline_neighbors(&all, "c", 4, "src");
        // order: c-1 (b), c+1 (d), c-2 (a), c+2 (e)
        assert_eq!(out.len(), 4);
        assert_eq!(out[0]["ref"]["stable_id"], "b");
        assert_eq!(out[1]["ref"]["stable_id"], "d");
        assert_eq!(out[2]["ref"]["stable_id"], "a");
        assert_eq!(out[3]["ref"]["stable_id"], "e");
        for item in &out {
            assert_eq!(item["ref"]["kind"], "section");
            assert_eq!(item["ref"]["source_id"], "src");
        }
    }

    #[test]
    fn outline_neighbors_returns_empty_when_anchor_missing() {
        let all = vec![section_row("a", "A", "A", 1, 1)];
        assert!(outline_neighbors(&all, "missing", 5, "src").is_empty());
    }

    #[test]
    fn outline_neighbors_stops_at_max() {
        let all: Vec<Value> = (0..10)
            .map(|i| section_row(&format!("s{i}"), "S", "S", i, 1))
            .collect();
        let out = outline_neighbors(&all, "s5", 2, "src");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn resolve_symbol_id_prefers_stable_id_over_others() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = codesight_storage::storage::IndexStore::open(tmp.path().join("x.sqlite"))?;
        let got = resolve_symbol_id(&store, Some("S"), Some("I"), Some("f"), Some(1), Some("n"))?;
        assert_eq!(got, Some("S".to_string()));
        Ok(())
    }

    #[test]
    fn resolve_symbol_id_falls_back_to_id() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = codesight_storage::storage::IndexStore::open(tmp.path().join("x.sqlite"))?;
        let got = resolve_symbol_id(&store, None, Some("I"), None, None, None)?;
        assert_eq!(got, Some("I".to_string()));
        Ok(())
    }

    #[test]
    fn resolve_symbol_id_returns_none_when_insufficient() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = codesight_storage::storage::IndexStore::open(tmp.path().join("x.sqlite"))?;
        assert!(resolve_symbol_id(&store, None, None, None, None, None)?.is_none());
        assert!(resolve_symbol_id(&store, Some(""), Some(""), None, None, None)?.is_none());
        assert!(resolve_symbol_id(&store, None, None, Some("f"), Some(0), None)?.is_none());
        // No DB rows — (path+name) and (name alone) both resolve to None
        // without erroring. Populated-store cases live in the storage crate.
        assert!(
            resolve_symbol_id(&store, None, None, Some("f.rs"), None, Some("no_such"))?.is_none()
        );
        assert!(resolve_symbol_id(&store, None, None, None, None, Some("no_such"))?.is_none());
        Ok(())
    }
}
