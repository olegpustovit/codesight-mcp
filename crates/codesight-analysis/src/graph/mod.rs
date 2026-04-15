//! Dependency graph algorithms: cycle detection, coupling metrics, layer
//! violations, call-graph traversal, PageRank, and PSR-4 FQN resolution.

mod calls;
mod coupling;
mod cycles;
mod layers;
mod pagerank;
mod psr4;

pub use calls::{
    CallEdge, SymbolInfo, build_call_edges, build_call_edges_including_heuristic, find_callees,
    find_callers, word_match,
};
pub use coupling::{CouplingMetrics, compute_coupling};
pub use cycles::find_cycles;
pub use layers::{LayerDef, LayerViolation, file_to_layer, find_layer_violations};
pub use pagerank::{InOutDegree, PageRankResult, compute_in_out_degrees, compute_pagerank};
pub use psr4::{Psr4Map, fqn_to_symbol, load_psr4_map, symbol_to_fqn};

#[cfg(test)]
mod tests;
