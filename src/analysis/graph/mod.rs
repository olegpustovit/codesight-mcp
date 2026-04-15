//! Dependency graph algorithms: cycle detection, coupling metrics, layer violations,
//! plus import graphs and resolution helpers.

mod calls;
mod coupling;
mod cycles;
mod import_graph;
mod layers;
mod pagerank;
mod psr4;

pub use codesight_imports::imports::{
    ImportEdge, extract_imports, language_for_path, load_tsconfig_aliases, parse_tsconfig_paths,
    resolve_import_edge_for_graph, resolve_imports_to_files, resolve_specifier,
};

pub use calls::{
    CallEdge, SymbolInfo, build_call_edges, build_call_edges_including_heuristic, find_callees,
    find_callers, word_match,
};
pub use coupling::{CouplingMetrics, compute_coupling};
pub use cycles::find_cycles;
pub use import_graph::{
    build_dependency_graph, build_dependency_graph_edges_with_aliases,
    build_dependency_graph_with_aliases,
};
pub use layers::{LayerDef, LayerViolation, file_to_layer, find_layer_violations};
pub use pagerank::{InOutDegree, PageRankResult, compute_in_out_degrees, compute_pagerank};
pub use psr4::{Psr4Map, fqn_to_symbol, load_psr4_map, symbol_to_fqn};

#[cfg(test)]
mod tests;
