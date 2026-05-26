//! Drift gate between `tools.json` (the authoritative MCP surface) and the
//! human-facing docs (`AGENTS.md`, `plugins/codesight-mcp/rules/codesight-routing.mdc`).
//!
//! What this guards against:
//! - The 6-tool wrapper drifting away from the documented 10-tool surface
//!   (the regression we just fixed). Any reappearance of `catalog_sources`,
//!   `inspect_graph`, `evaluate_quality`, etc. fails CI.
//! - Variant enums (`mode=foo`, `view=foo`, `kind=foo`, `analysis=foo`,
//!   `graph=foo`) referenced in the docs without a backing `tools.json` enum
//!   entry.
//! - Warning codes referenced in the docs that aren't declared in
//!   `agent_v1::warnings::ALL_CODES`.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use codesight_engine::service::agent_v1::warnings::ALL_CODES as WARNING_CODES;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_doc(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn tools_json() -> Vec<serde_json::Value> {
    let raw = read_doc("crates/codesight-engine/src/tools.json");
    serde_json::from_str(&raw).expect("tools.json is valid JSON")
}

/// Returns:
/// - the set of public tool names declared in `tools.json`
/// - the set of `(variant_key, variant_value)` pairs declared by *any* tool's
///   schema enum (e.g. `("mode", "regex")`).
fn canonical_surface() -> (HashSet<String>, HashSet<(String, String)>) {
    let mut names = HashSet::new();
    let mut variants = HashSet::new();
    for tool in tools_json() {
        let name = tool["name"]
            .as_str()
            .expect("tool name is a string")
            .to_string();
        names.insert(name);
        if let serde_json::Value::Object(props) = &tool["inputSchema"]["properties"] {
            for (key, schema) in props {
                if let Some(serde_json::Value::Array(values)) = schema.get("enum") {
                    for v in values {
                        if let Some(s) = v.as_str() {
                            variants.insert((key.clone(), s.to_string()));
                        }
                    }
                }
            }
        }
    }
    (names, variants)
}

/// Tool names from the prior 10-tool surface that **must not** reappear in the
/// docs. The historical churn was: this list shrank to 6, but the docs kept
/// referencing the old names long after.
const RETIRED_TOOL_NAMES: &[&str] = &[
    "catalog_sources",
    "manage_indexes",
    "search_catalog",
    "inspect_structure",
    "inspect_graph",
    "read_content",
    "evaluate_impact",
    "evaluate_quality",
    "assemble_context",
    "ingest_sources",
    "find_references",
    "find_importers",
];

/// Variant keys we treat as "must reference a real schema enum value".
/// Other args (`limit=`, `top_n=`, etc.) are caller-tunable parameters, not
/// dispatch enums, so they're not enforced here.
const VARIANT_KEYS: &[&str] = &["mode", "view", "kind", "analysis", "graph"];

const DOCS: &[&str] = &[
    "AGENTS.md",
    "plugins/codesight-mcp/rules/codesight-routing.mdc",
];

#[test]
fn no_retired_tool_names_in_docs() {
    for doc_path in DOCS {
        let body = read_doc(doc_path);
        for retired in RETIRED_TOOL_NAMES {
            // Backtick-quoted form: `name`. We deliberately tolerate the
            // word appearing in prose (e.g. historical ROADMAP references).
            let backticked = format!("`{retired}`");
            assert!(
                !body.contains(&backticked),
                "{doc_path} still references retired tool name `{retired}` — see tools.json for the current 6-tool surface"
            );
        }
    }
}

#[test]
fn variant_pairs_in_docs_match_tools_json() {
    let (_, variants) = canonical_surface();
    for doc_path in DOCS {
        let body = read_doc(doc_path);
        for (key, value) in extract_variant_pairs(&body) {
            assert!(
                variants.contains(&(key.clone(), value.clone())),
                "{doc_path} references `{key}={value}` but tools.json declares no such enum value. \
                 Allowed values for `{key}=`: {:?}",
                variants
                    .iter()
                    .filter(|(k, _)| k == &key)
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>(),
            );
        }
    }
}

#[test]
fn warning_codes_in_docs_are_declared() {
    let declared: HashSet<&str> = WARNING_CODES.iter().copied().collect();
    for doc_path in DOCS {
        let body = read_doc(doc_path);
        for code in extract_warning_codes(&body) {
            assert!(
                declared.contains(code.as_str()),
                "{doc_path} mentions warning code `{code}` but it isn't declared in \
                 codesight_engine::service::agent_v1::warnings::ALL_CODES"
            );
        }
    }
}

#[test]
fn every_tool_in_tools_json_appears_in_routing_skill() {
    // Reverse drift: a new tool added to tools.json must show up in the
    // routing skill so callers know it exists.
    let (names, _) = canonical_surface();
    let body = read_doc("plugins/codesight-mcp/rules/codesight-routing.mdc");
    for name in &names {
        let backticked = format!("`{name}`");
        assert!(
            body.contains(&backticked),
            "routing.mdc never mentions tool `{name}` (declared in tools.json). \
             Add a row to the routing table."
        );
    }
}

// ---------- text scanning helpers ----------

/// Yields every `key=value` pair in `body` where `key` is one of the
/// dispatch-enum keys we care about. The `value` is the longest run of
/// `[A-Za-z0-9_]` immediately following the `=`.
fn extract_variant_pairs(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in VARIANT_KEYS {
        let needle = format!("{key}=");
        let mut search_from = 0;
        while let Some(rel) = body[search_from..].find(&needle) {
            let start = search_from + rel + needle.len();
            let value: String = body[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !value.is_empty() {
                out.push((key.to_string(), value));
            }
            search_from = start.max(search_from + 1);
        }
    }
    out
}

/// Yields every backtick-quoted SCREAMING_SNAKE_CASE identifier in `body`.
/// Heuristic — over-matches on things like `OK`, but those happen not to be
/// in our warning code list, so the assertion still works in practice.
fn extract_warning_codes(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, c) in body.char_indices() {
        if c != '`' {
            continue;
        }
        let token: String = body[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        if token.len() >= 5 && token.contains('_') && body[i + 1 + token.len()..].starts_with('`') {
            out.push(token);
        }
    }
    out
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn extract_variant_pairs_finds_inline_examples() {
        let pairs = extract_variant_pairs("Use `mode=keyword` and `analysis=hotspots`.");
        assert!(pairs.contains(&("mode".to_string(), "keyword".to_string())));
        assert!(pairs.contains(&("analysis".to_string(), "hotspots".to_string())));
    }

    #[test]
    fn extract_warning_codes_picks_codes_only() {
        let codes = extract_warning_codes("expect `OUTLINE_FIRST` and `TOKEN_HEAVY_READ` warnings");
        assert!(codes.iter().any(|c| c == "OUTLINE_FIRST"));
        assert!(codes.iter().any(|c| c == "TOKEN_HEAVY_READ"));
    }
}
