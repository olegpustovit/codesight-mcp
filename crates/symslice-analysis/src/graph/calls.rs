//! Heuristic call graph from symbol bodies (whole-word name matches).
//!
//! By default, edges are built only between symbols whose extraction tier maps to
//! [`symslice_core::symbols::FactQuality::suitable_for_strict_analysis`]. Heuristic-tier
//! symbols remain available for exploratory graphs via [`build_call_edges_including_heuristic`].

use rustc_hash::{FxHashMap, FxHashSet};

use serde::Serialize;

use symslice_core::symbols::ExtractionTier;
use symslice_language_support::text_match::iter_identifiers;
use symslice_storage::storage::SymbolSourceRecord;

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
    // share a name (e.g. every `new`); they all participate as fallback
    // candidates when no same-file definition exists.
    //
    // Path-anchored fix (P4.4): when a caller's source mentions an
    // identifier and the caller's *own file* defines a symbol with that
    // name, restrict the edge set to those same-file definitions. Without
    // this, every caller in the repo whose body mentioned `dispatch` got
    // edges to all 10+ functions named `dispatch` across the codebase —
    // the bundle and find_callers paths bled across module boundaries
    // even though same-file resolution is overwhelmingly the right answer.
    //
    // Bare-name lookup remains the fallback when no same-file match exists
    // (the caller is genuinely calling into another module).
    let mut name_to_callees: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    let mut file_name_to_callees: FxHashMap<(&str, &str), Vec<usize>> = FxHashMap::default();
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
        file_name_to_callees
            .entry((callee.file.as_str(), callee.name.as_str()))
            .or_default()
            .push(i);
    }

    let mut edges = Vec::new();
    // Reused across callers — one allocation per build, not per caller.
    let mut seen_callees: FxHashSet<usize> = FxHashSet::default();
    for caller in symbols {
        if caller.source.is_empty() {
            continue;
        }
        if !symbol_ok_for_policy(caller.extraction_tier, policy) {
            continue;
        }
        seen_callees.clear();
        for ident in iter_identifiers(&caller.source) {
            // Prefer same-file callees: when the caller's file defines a
            // symbol with this name, only those participate. Fall back to
            // the workspace-wide list when no same-file match exists.
            let same_file = file_name_to_callees.get(&(caller.file.as_str(), ident));
            let resolved_indices = match same_file {
                Some(indices) => Some(indices),
                None => name_to_callees.get(ident),
            };
            let Some(indices) = resolved_indices else {
                continue;
            };
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

    edges
}

/// Build call edges from a list of symbols by checking if a symbol's body
/// text contains another symbol's name as a whole-word match.
///
/// Only symbols whose tier maps to [`symslice_core::symbols::FactQuality::suitable_for_strict_analysis`]
/// participate as callers or callees (REFACTOR_PLAN: graph analysis defaults to trusted/baseline structural).
#[must_use]
pub fn build_call_edges(symbols: &[SymbolInfo]) -> Vec<CallEdge> {
    build_call_edges_with_policy(symbols, CallEdgePolicy::StrictAnalysis)
}

/// Same as [`build_call_edges`], but allows [`symslice_core::symbols::FactQuality::Heuristic`]
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
    let mut visited = FxHashSet::default();
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
    let mut visited = FxHashSet::default();
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
    use symslice_core::symbols::ExtractionTier;

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
