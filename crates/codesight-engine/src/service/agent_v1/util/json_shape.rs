//! Symbol / section envelope shaping, score rounding, and symbol-ref resolution.

use serde_json::{Value, json};

use codesight_storage::storage::{SearchSymbol, doc_index::SearchSection};

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

pub(in crate::service) fn doc_format_to_type_label(format: &str) -> String {
    match format {
        "markdown" => ".md".to_string(),
        "text" => ".txt".to_string(),
        "rst" => ".rst".to_string(),
        "asciidoc" => ".adoc".to_string(),
        other => format!(".{other}"),
    }
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
        assert!(
            resolve_symbol_id(&store, None, None, Some("f.rs"), None, Some("no_such"))?.is_none()
        );
        assert!(resolve_symbol_id(&store, None, None, None, None, Some("no_such"))?.is_none());
        Ok(())
    }
}
