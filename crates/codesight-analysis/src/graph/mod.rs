//! Dependency graph algorithms: cycle detection, coupling metrics, layer
//! violations, call-graph traversal, PageRank. Namespace/FQN resolution lives
//! per-language on the `Language::resolve_fqn` trait method.

mod calls;
mod coupling;
mod cycles;
mod layers;
mod pagerank;
mod usage_edges;

pub use calls::{
    CallEdge, SymbolInfo, build_call_edges, build_call_edges_including_heuristic, find_callees,
    find_callers,
};
pub use coupling::{CouplingMetrics, compute_coupling};
pub use cycles::find_cycles;
pub use layers::{LayerDef, LayerViolation, file_to_layer, find_layer_violations};
pub use pagerank::{InOutDegree, PageRankResult, compute_in_out_degrees, compute_pagerank};
pub use usage_edges::{adjacency_with_usage_edges, build_symbol_usage_adjacency, union_adjacency};

#[cfg(test)]
mod tests;
