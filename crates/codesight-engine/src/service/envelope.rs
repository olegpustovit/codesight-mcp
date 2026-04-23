//! Response-envelope helpers: `compact_meta` and
//! `extract_read_content_paths`. Extracted from `service/mod.rs` so the
//! `CodeService` impl stays legible.

use serde_json::{Map, Value, json};

/// Collect file paths whose content was returned by a successful
/// `read_content` call. Walks the `data.items[].ref.path` shape.
pub(super) fn extract_read_content_paths(response: &Value) -> Vec<String> {
    let Some(items) = response
        .pointer("/data/items")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if let Some(path) = item
            .pointer("/ref/path")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(path.to_string());
        }
    }
    out
}

/// Paths returned via a *full-file* `read_content` ref — everything else
/// (symbols, sections, bounded ranges) is excluded. Used by the session
/// ledger to run the repeated-full-file nudge without charging narrower
/// reads.
pub(super) fn extract_full_file_read_paths(response: &Value) -> Vec<String> {
    let Some(items) = response
        .pointer("/data/items")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let is_full_file = item
            .get("read_scope")
            .and_then(Value::as_str)
            .is_some_and(|s| s == "file" || s == "file_full");
        if !is_full_file {
            continue;
        }
        if let Some(path) = item
            .pointer("/ref/path")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(path.to_string());
        }
    }
    out
}

pub(super) fn compact_meta(meta: &mut Map<String, Value>) {
    // Keys always kept when present.
    const ALWAYS: &[&str] = &[
        "timing_ms",
        "tokens_saved",
        "error_category",
        "source",
        "methodology",
        "confidence",
        "confidence_level",
        "suggestion",
        "next_step_hint",
    ];
    // Boolean flags kept only when true.
    const TRUE_ONLY: &[&str] = &[
        "fuzzy_applied",
        "semantic_applied",
        "truncated",
        "domain_explicit",
    ];

    let default_unit_kinds = json!({
        "symbol": true,
        "section": false,
        "file": false,
    });
    let mut minimal = Map::new();

    for &key in ALWAYS {
        if let Some(value) = meta.get(key).cloned() {
            minimal.insert(key.to_string(), value);
        }
    }
    for &key in TRUE_ONLY {
        if meta.get(key).and_then(Value::as_bool) == Some(true)
            && let Some(value) = meta.get(key).cloned()
        {
            minimal.insert(key.to_string(), value);
        }
    }

    // Keys kept only when they differ from a noisy default.
    type KeepFn = fn(&Value) -> bool;
    let nondefault_unit_kinds = |v: &Value| v != &default_unit_kinds;
    let nondefault: [(&str, KeepFn); 3] = [
        ("query_mode_effective", |v| v != &Value::String(String::from("keyword"))),
        ("domain_effective", |v| v != &Value::String(String::from("code"))),
        ("truncated_by", |v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false)),
    ];
    for (key, keep) in nondefault {
        if let Some(value) = meta.get(key).cloned()
            && keep(&value)
        {
            minimal.insert(key.to_string(), value);
        }
    }
    if let Some(value) = meta.get("unit_kinds_effective").cloned()
        && nondefault_unit_kinds(&value)
    {
        minimal.insert("unit_kinds_effective".to_string(), value);
    }

    *meta = minimal;
}
