//! SQL import extraction — dbt-style Jinja `{{ ref(...) }}` /
//! `{{ source('s', 't') }}` references. Hand-rolled scanner because dbt's
//! Jinja interpolation isn't reachable via the SQL tree-sitter grammar.

use rustc_hash::FxHashSet;

use crate::imports_types::{ImportEdge, extract_quoted_string, extract_quoted_string_with_rest};

pub fn extract(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();
    let mut seen = FxHashSet::default();

    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut index = 0;

    while index + 1 < len {
        if bytes[index] == b'{' && bytes[index + 1] == b'{' {
            let block_start = index + 2;
            if let Some(end_offset) = content[block_start..].find("}}") {
                let block = &content[block_start..block_start + end_offset];
                let block_trimmed = block
                    .trim()
                    .trim_start_matches('-')
                    .trim()
                    .trim_end_matches('-')
                    .trim();

                if let Some(after_ref) = block_trimmed.strip_prefix("ref(") {
                    if let Some(name) = extract_quoted_string(after_ref) {
                        let specifier = name.to_string();
                        if !specifier.is_empty() && seen.insert(specifier.clone()) {
                            edges.push(ImportEdge {
                                specifier,
                                names: Vec::new(),
                            });
                        }
                    }
                } else if let Some(after_source) = block_trimmed.strip_prefix("source(")
                    && let Some((schema, rest)) = extract_quoted_string_with_rest(after_source)
                    && let Some(table) = extract_quoted_string(rest)
                {
                    let specifier = format!("source:{schema}.{table}");
                    if seen.insert(specifier.clone()) {
                        edges.push(ImportEdge {
                            specifier,
                            names: Vec::new(),
                        });
                    }
                }
                index = block_start + end_offset + 2;
            } else {
                index += 2;
            }
        } else {
            index += 1;
        }
    }

    edges
}
