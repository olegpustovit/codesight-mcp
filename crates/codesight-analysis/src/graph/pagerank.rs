//! PageRank and degree metrics on file dependency graphs.
//!
//! Thin adapter over `petgraph::algo::page_rank` and petgraph's directed-edge
//! degree iterators. petgraph's PageRank handles dangling-node mass
//! redistribution internally, so output sums to ~1.0 like the previous
//! hand-rolled impl.

use petgraph::Direction;
use petgraph::Graph;
use petgraph::algo::page_rank;
use petgraph::graph::NodeIndex;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

/// Result of a PageRank computation.
#[derive(Debug, Clone, Serialize)]
pub struct PageRankResult {
    /// Node → score (sums to ~1.0).
    pub scores: FxHashMap<String, f64>,
    /// Number of iterations executed.
    pub iterations: usize,
}

/// In-degree and out-degree for a node.
#[derive(Debug, Clone, Serialize)]
pub struct InOutDegree {
    pub in_degree: usize,
    pub out_degree: usize,
}

/// Build a `petgraph::Graph` from an adjacency list, skipping self-loops and
/// deduping parallel edges. Returns the graph and a sorted node list whose
/// position equals the corresponding `NodeIndex::index()`.
fn build_graph(
    adj: &FxHashMap<String, Vec<String>>,
) -> (Graph<(), ()>, Vec<String>, FxHashMap<String, NodeIndex>) {
    let mut nodes_set: FxHashSet<&str> = adj.keys().map(String::as_str).collect();
    for deps in adj.values() {
        for d in deps {
            nodes_set.insert(d.as_str());
        }
    }
    let mut nodes: Vec<String> = nodes_set.into_iter().map(String::from).collect();
    nodes.sort();

    let mut graph = Graph::<(), ()>::new();
    let mut idx: FxHashMap<String, NodeIndex> = FxHashMap::default();
    for n in &nodes {
        idx.insert(n.clone(), graph.add_node(()));
    }

    for (src, deps) in adj {
        let si = idx[src];
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        for dep in deps {
            if dep == src || !seen.insert(dep.as_str()) {
                continue;
            }
            if let Some(&di) = idx.get(dep) {
                graph.add_edge(si, di, ());
            }
        }
    }

    (graph, nodes, idx)
}

/// Compute in-degree and out-degree for every node in the adjacency list.
/// Self-loops are excluded, parallel edges deduped.
pub fn compute_in_out_degrees(
    adj: &FxHashMap<String, Vec<String>>,
) -> FxHashMap<String, InOutDegree> {
    let (graph, nodes, idx) = build_graph(adj);
    nodes
        .into_iter()
        .map(|n| {
            let i = idx[&n];
            let in_degree = graph.edges_directed(i, Direction::Incoming).count();
            let out_degree = graph.edges_directed(i, Direction::Outgoing).count();
            (
                n,
                InOutDegree {
                    in_degree,
                    out_degree,
                },
            )
        })
        .collect()
}

/// Compute PageRank on a directed graph.
///
/// - `adj`: adjacency list (source → [targets])
/// - `damping`: damping factor, typically 0.85
/// - `max_iter`: number of iterations to run
///
/// Self-loops and parallel edges are ignored. Returns scores summing to ~1.0
/// (petgraph redistributes dangling-node mass each iteration).
pub fn compute_pagerank(
    adj: &FxHashMap<String, Vec<String>>,
    damping: f64,
    max_iter: usize,
    _tol: f64,
) -> PageRankResult {
    if adj.is_empty() {
        return PageRankResult {
            scores: FxHashMap::default(),
            iterations: 0,
        };
    }
    let (graph, nodes, idx) = build_graph(adj);
    if graph.node_count() == 0 {
        return PageRankResult {
            scores: FxHashMap::default(),
            iterations: 0,
        };
    }
    let raw = page_rank(&graph, damping, max_iter);
    let scores = nodes
        .into_iter()
        .map(|n| {
            let i = idx[&n].index();
            (n, raw[i])
        })
        .collect();
    PageRankResult {
        scores,
        iterations: max_iter,
    }
}
