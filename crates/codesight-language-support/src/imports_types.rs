//! Shared types for the `Language::extract_imports` trait contract.

use serde::{Deserialize, Serialize};

/// A structured import edge extracted from source code.
///
/// `specifier` is the module path up to (but excluding) the final leaf;
/// `names` holds the leaf identifiers being brought into scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportEdge {
    pub specifier: String,
    pub names: Vec<String>,
}

/// Return a UTF-8 slice of `source` corresponding to a tree-sitter node.
pub fn source_slice<'a>(node: tree_sitter::Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.byte_range())
}

/// Dedupe a vec of edges, keeping first occurrence order; merges `names`
/// when the same `specifier` appears multiple times.
pub fn deduplicate_edges(edges: Vec<ImportEdge>) -> Vec<ImportEdge> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for edge in edges {
        let entry = seen.entry(edge.specifier.clone()).or_insert_with(|| {
            order.push(edge.specifier.clone());
            Vec::new()
        });
        for name in &edge.names {
            if !entry.contains(name) {
                entry.push(name.clone());
            }
        }
    }
    order
        .into_iter()
        .map(|specifier| ImportEdge {
            names: seen.remove(&specifier).unwrap_or_default(),
            specifier,
        })
        .collect()
}

/// Extract the innermost `'…'` or `"…"` string from `s`.
pub fn extract_quoted_string(s: &str) -> Option<&str> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b { (a, '\'') } else { (b, '"') }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

/// Default display format — ES-module-style `import { A, B } from 'spec'`.
/// Special-cases bare relative / `source:` specifiers to emit `import spec`.
pub fn format_esm_import_lines(edges: &[ImportEdge]) -> Vec<String> {
    let mut lines = Vec::new();
    for e in edges {
        if e.names.is_empty() {
            if e.specifier.starts_with('.') || e.specifier.starts_with("source:") {
                lines.push(format!("import {}", e.specifier));
            } else if e.specifier.contains('/') || e.specifier.starts_with('@') {
                lines.push(format!("import '{}'", e.specifier.replace('\'', "")));
            } else {
                lines.push(format!("import {}", e.specifier));
            }
        } else {
            lines.push(format!(
                "import {{ {} }} from '{}'",
                e.names.join(", "),
                e.specifier.replace('\'', "")
            ));
        }
    }
    lines
}

pub fn extract_quoted_string_with_rest(s: &str) -> Option<(&str, &str)> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b { (a, '\'') } else { (b, '"') }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some((&rest[..end], &rest[end + 1..]))
}
