//! Heuristic call graph from symbol bodies (whole-word name matches).
//!
//! By default, edges are built only between symbols whose extraction tier maps to
//! [`codesight_core::symbols::FactQuality::suitable_for_strict_analysis`]. Heuristic-tier
//! symbols remain available for exploratory graphs via [`build_call_edges_including_heuristic`].

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use codesight_core::symbols::ExtractionTier;
use codesight_language_support::text_match::iter_identifiers;
use codesight_storage::storage::SymbolSourceRecord;

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

pub type SymbolInfo = SymbolSourceRecord;

#[derive(Clone, Copy)]
enum CallEdgePolicy {
    /// Exclude edges where either endpoint is [`FactQuality::Heuristic`].
    StrictAnalysis,
    /// Include all resolvable symbol pairs (legacy / exploratory).
    Exploratory,
}

fn symbol_ok_for_policy(tier: ExtractionTier, policy: CallEdgePolicy) -> bool {
    match policy {
        CallEdgePolicy::Exploratory => true,
        CallEdgePolicy::StrictAnalysis => tier.fact_quality().suitable_for_strict_analysis(),
    }
}

fn build_call_edges_with_policy(symbols: &[SymbolInfo], policy: CallEdgePolicy) -> Vec<CallEdge> {
    // Build a `name → callee-indices` map once (O(N)). Multiple callees can
    // share a name (e.g. every `new`); they all participate.
    //
    // Replaces a naive O(N²) loop — for a 4.5k-symbol repo the old version
    // did 20M substring searches and blew past the 2000 ms handler cap on
    // `graph=calls` and `evaluate_impact analysis=impact_preview`.
    let mut name_to_callees: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, callee) in symbols.iter().enumerate() {
        if !symbol_ok_for_policy(callee.extraction_tier, policy) {
            continue;
        }
        if callee.name.len() < 2 {
            continue;
        }
        name_to_callees
            .entry(callee.name.as_str())
            .or_default()
            .push(i);
    }

    let mut edges = Vec::new();
    // Reused across callers — one allocation per build, not per caller.
    let mut seen_callees: HashSet<usize> = HashSet::new();
    for caller in symbols {
        if caller.source.is_empty() {
            continue;
        }
        if !symbol_ok_for_policy(caller.extraction_tier, policy) {
            continue;
        }
        // Walk identifiers in the caller's source. Each maximal run of
        // `[A-Za-z0-9_]` is a candidate callee name — O(source_len) to
        // scan, O(1) hash lookup per identifier. Dedup per caller so we
        // match the old word_match-bool behavior (one edge per pair).
        seen_callees.clear();
        for ident in iter_identifiers(&caller.source) {
            if let Some(indices) = name_to_callees.get(ident) {
                for &ci in indices {
                    let callee = &symbols[ci];
                    if callee.id == caller.id {
                        continue;
                    }
                    if !seen_callees.insert(ci) {
                        continue;
                    }
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
    }

    edges
}

/// Build call edges from a list of symbols by checking if a symbol's body
/// text contains another symbol's name as a whole-word match.
///
/// Only symbols whose tier maps to [`codesight_core::symbols::FactQuality::suitable_for_strict_analysis`]
/// participate as callers or callees (REFACTOR_PLAN: graph analysis defaults to trusted/baseline structural).
#[must_use]
pub fn build_call_edges(symbols: &[SymbolInfo]) -> Vec<CallEdge> {
    build_call_edges_with_policy(symbols, CallEdgePolicy::StrictAnalysis)
}

/// Same as [`build_call_edges`], but allows [`codesight_core::symbols::FactQuality::Heuristic`]
/// symbols as edge endpoints (exploratory / legacy behavior).
#[must_use]
pub fn build_call_edges_including_heuristic(symbols: &[SymbolInfo]) -> Vec<CallEdge> {
    build_call_edges_with_policy(symbols, CallEdgePolicy::Exploratory)
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

#[cfg(test)]
mod tests {
    use codesight_core::symbols::ExtractionTier;

    use super::{SymbolInfo, build_call_edges, build_call_edges_including_heuristic};

    fn sym(id: &str, name: &str, source: &str, tier: ExtractionTier) -> SymbolInfo {
        SymbolInfo {
            id: id.to_string(),
            name: name.to_string(),
            kind: "function".to_string(),
            file: "a.ts".to_string(),
            line: 1,
            end_line: 2,
            source: source.to_string(),
            extraction_tier: tier,
        }
    }

    #[test]
    fn strict_call_graph_drops_edges_involving_heuristic_tier() {
        let symbols = vec![
            sym("1", "caller", "foo()", ExtractionTier::CuratedQueryPack),
            sym("2", "foo", "// body", ExtractionTier::ManualOverride),
        ];
        assert!(
            build_call_edges(&symbols).is_empty(),
            "callee is heuristic-only"
        );

        let symbols2 = vec![
            sym("1", "caller", "foo()", ExtractionTier::ManualOverride),
            sym("2", "foo", "// body", ExtractionTier::CuratedQueryPack),
        ];
        assert!(
            build_call_edges(&symbols2).is_empty(),
            "caller is heuristic-only"
        );
    }

    #[test]
    fn strict_call_graph_keeps_baseline_structural_endpoints() {
        let symbols = vec![
            sym("1", "caller", "foo()", ExtractionTier::GenericAstFallback),
            sym("2", "foo", "// body", ExtractionTier::Profiled),
        ];
        let edges = build_call_edges(&symbols);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].caller_id, "1");
        assert_eq!(edges[0].callee_id, "2");
    }

    #[test]
    fn exploratory_includes_heuristic_endpoints() {
        let symbols = vec![
            sym("1", "caller", "foo()", ExtractionTier::CuratedQueryPack),
            sym("2", "foo", "// body", ExtractionTier::ManualOverride),
        ];
        let edges = build_call_edges_including_heuristic(&symbols);
        assert_eq!(edges.len(), 1);
    }
}
