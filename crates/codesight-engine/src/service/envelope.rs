//! Response-envelope helpers: `compact_meta`, `panic_message`,
//! `extract_read_content_paths`, and the `requested_*` arg accessors.
//! Extracted from `service/mod.rs` so the `CodeService` impl stays legible.

use serde_json::{Map, Value, json};

pub(super) fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

pub(super) fn requested_bool(arguments: &Value, key: &str) -> bool {
    match arguments.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

pub(super) fn requested_string<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

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
