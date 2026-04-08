use std::collections::HashMap;

use crate::indexing::imports::ImportEdge;

pub(crate) fn deduplicate_edges(edges: Vec<ImportEdge>) -> Vec<ImportEdge> {
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

pub(crate) fn extract_quoted_string(s: &str) -> Option<&str> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b {
                (a, '\'')
            } else {
                (b, '"')
            }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

pub(crate) fn extract_quoted_string_with_rest(s: &str) -> Option<(&str, &str)> {
    let start_single = s.find('\'');
    let start_double = s.find('"');
    let (start, quote) = match (start_single, start_double) {
        (Some(a), Some(b)) => {
            if a < b {
                (a, '\'')
            } else {
                (b, '"')
            }
        }
        (Some(a), None) => (a, '\''),
        (None, Some(b)) => (b, '"'),
        (None, None) => return None,
    };
    let rest = &s[start + 1..];
    let end = rest.find(quote)?;
    Some((&rest[..end], &rest[end + 1..]))
}
