//! Heuristic call graph from symbol bodies (whole-word name matches).

use std::collections::HashSet;

use serde::Serialize;

/// Check if `body` contains `name` as a whole-word match (not a partial match).
/// Uses simple word-boundary heuristic: the character before and after `name`
/// must not be alphanumeric or underscore.
pub fn word_match(body: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let name_bytes = name.as_bytes();
    let body_bytes = body.as_bytes();
    let nlen = name_bytes.len();

    let mut start = 0;
    while start + nlen <= body_bytes.len() {
        if let Some(pos) = body[start..].find(name) {
            let abs_pos = start + pos;
            let before_ok = abs_pos == 0 || !is_word_char(body_bytes[abs_pos - 1]);
            let after_ok =
                abs_pos + nlen >= body_bytes.len() || !is_word_char(body_bytes[abs_pos + nlen]);
            if before_ok && after_ok {
                return true;
            }
            start = abs_pos + 1;
        } else {
            break;
        }
    }
    false
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A caller/callee edge in the call graph.
#[derive(Debug, Clone, Serialize)]
pub struct CallEdge {
    pub caller_id: String,
    pub caller_name: String,
    pub caller_file: String,
    pub caller_line: u32,
    pub caller_kind: String,
    pub callee_id: String,
    pub callee_name: String,
    pub callee_file: String,
    pub callee_line: u32,
    pub callee_kind: String,
}

/// Symbol info for call graph building.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub end_line: u32,
    pub source: String,
}

/// Build call edges from a list of symbols by checking if a symbol's body
/// text contains another symbol's name as a whole-word match.
pub fn build_call_edges(symbols: &[SymbolInfo]) -> Vec<CallEdge> {
    let mut edges = Vec::new();

    for caller in symbols {
        if caller.source.is_empty() {
            continue;
        }
        for callee in symbols {
            if caller.id == callee.id {
                continue;
            }
            // Skip very short names to avoid false positives
            if callee.name.len() < 2 {
                continue;
            }
            if word_match(&caller.source, &callee.name) {
                edges.push(CallEdge {
                    caller_id: caller.id.clone(),
                    caller_name: caller.name.clone(),
                    caller_file: caller.file.clone(),
                    caller_line: caller.line,
                    caller_kind: caller.kind.clone(),
                    callee_id: callee.id.clone(),
                    callee_name: callee.name.clone(),
                    callee_file: callee.file.clone(),
                    callee_line: callee.line,
                    callee_kind: callee.kind.clone(),
                });
            }
        }
    }

    edges
}

/// Find all callers (direct and transitive up to `max_depth`) for a given symbol.
/// Returns (caller_id, caller_name, caller_file, caller_line, caller_kind, depth).
pub fn find_callers(
    symbol_id: &str,
    edges: &[CallEdge],
    max_depth: usize,
) -> Vec<(String, String, String, u32, String, usize)> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(symbol_id.to_string());
    let mut frontier: Vec<(String, usize)> = vec![(symbol_id.to_string(), 0)];

    while let Some((current, depth)) = frontier.pop() {
        if depth >= max_depth {
            continue;
        }
        for edge in edges {
            if edge.callee_id == current && !visited.contains(&edge.caller_id) {
                visited.insert(edge.caller_id.clone());
                result.push((
                    edge.caller_id.clone(),
                    edge.caller_name.clone(),
                    edge.caller_file.clone(),
                    edge.caller_line,
                    edge.caller_kind.clone(),
                    depth + 1,
                ));
                frontier.push((edge.caller_id.clone(), depth + 1));
            }
        }
    }

    result
}

/// Find all callees (direct and transitive up to `max_depth`) for a given symbol.
pub fn find_callees(
    symbol_id: &str,
    edges: &[CallEdge],
    max_depth: usize,
) -> Vec<(String, String, String, u32, String, usize)> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(symbol_id.to_string());
    let mut frontier: Vec<(String, usize)> = vec![(symbol_id.to_string(), 0)];

    while let Some((current, depth)) = frontier.pop() {
        if depth >= max_depth {
            continue;
        }
        for edge in edges {
            if edge.caller_id == current && !visited.contains(&edge.callee_id) {
                visited.insert(edge.callee_id.clone());
                result.push((
                    edge.callee_id.clone(),
                    edge.callee_name.clone(),
                    edge.callee_file.clone(),
                    edge.callee_line,
                    edge.callee_kind.clone(),
                    depth + 1,
                ));
                frontier.push((edge.callee_id.clone(), depth + 1));
            }
        }
    }

    result
}
