//! Generic SCM-driven import extractor.
//!
//! Each language ships `queries/imports.scm` that tags nodes with
//! `@specifier` (the import path string) and `@name` (each imported name).
//! This module runs the query and emits `ImportEdge`s — zero per-lang Rust
//! code needed. Post-processing groups same-specifier rows together via
//! [`deduplicate_edges`].
//!
//! Languages whose import structure can't be expressed declaratively (Rust,
//! whose nested `use a::{b::C, d}` trees resist simple capture grouping)
//! override `Language::extract_imports` with a hand-rolled walker.

use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::frameworks::parse_tree;
use crate::imports_types::{ImportEdge, deduplicate_edges};

/// Run `lang.imports_query()` against `source` parsed with `lang.grammar()`
/// and return the flattened + deduped import edges. Returns `None` when the
/// grammar or query is missing, the source fails to parse, or the parse
/// tree has syntax errors.
pub fn extract(
    source: &str,
    grammar: tree_sitter::Language,
    query_src: &str,
) -> Option<Vec<ImportEdge>> {
    let tree = parse_tree(grammar.clone(), source)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let query = Query::new(&grammar, query_src).ok()?;
    let mut cursor = QueryCursor::new();
    let specifier_idx = query.capture_index_for_name("specifier");
    let name_idx = query.capture_index_for_name("name");
    let bytes = source.as_bytes();

    let mut edges = Vec::new();
    let mut matches = cursor.matches(&query, root, bytes);
    while let Some(m) = matches.next() {
        let mut specifier: Option<String> = None;
        let mut names: Vec<String> = Vec::new();
        for cap in m.captures {
            let idx = cap.index;
            if Some(idx) == specifier_idx
                && let Ok(text) = cap.node.utf8_text(bytes)
            {
                specifier = Some(unquote(text).to_string());
            } else if Some(idx) == name_idx
                && let Ok(text) = cap.node.utf8_text(bytes)
            {
                names.push(text.to_string());
            }
        }
        if let Some(spec) = specifier {
            edges.push(ImportEdge {
                specifier: spec,
                names,
            });
        }
    }

    Some(deduplicate_edges(edges))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
            || (b[0] == b'`' && b[b.len() - 1] == b'`'))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}
