//! PageRank and degree metrics on file dependency graphs.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// Result of a PageRank computation.
#[derive(Debug, Clone, Serialize)]
pub struct PageRankResult {
    /// Node → score (sums to ~1.0).
    pub scores: HashMap<String, f64>,
    /// Number of iterations until convergence.
    pub iterations: usize,
}

/// In-degree and out-degree for a node.
#[derive(Debug, Clone, Serialize)]
pub struct InOutDegree {
    pub in_degree: usize,
    pub out_degree: usize,
}

/// Compute in-degree and out-degree for every node in the adjacency list.
/// Self-loops are excluded.
pub fn compute_in_out_degrees(adj: &HashMap<String, Vec<String>>) -> HashMap<String, InOutDegree> {
    let mut result: HashMap<String, InOutDegree> = HashMap::new();

    // Initialize all nodes
    for node in adj.keys() {
        result.entry(node.clone()).or_insert(InOutDegree {
            in_degree: 0,
            out_degree: 0,
        });
    }

    for (source, deps) in adj {
        let mut seen = HashSet::new();
        for dep in deps {
            if dep == source {
                continue; // skip self-loops
            }
            if seen.insert(dep.clone()) {
                result
                    .entry(source.clone())
                    .or_insert(InOutDegree {
                        in_degree: 0,
                        out_degree: 0,
                    })
                    .out_degree += 1;
                result
                    .entry(dep.clone())
                    .or_insert(InOutDegree {
                        in_degree: 0,
                        out_degree: 0,
                    })
                    .in_degree += 1;
            }
        }
    }

    // Fix out_degree: we incremented per unique dep, but we need to also
    // count from adj directly (excluding self-loops and deduping).
    // Actually the above already does this correctly via `seen`. Let's recount.
    for (source, deps) in adj {
        let unique_out: HashSet<_> = deps.iter().filter(|d| *d != source).collect();
        if let Some(entry) = result.get_mut(source) {
            entry.out_degree = unique_out.len();
        }
    }

    result
}

/// Compute PageRank on a directed graph.
///
/// - `adj`: adjacency list (source → [targets])
/// - `damping`: damping factor, typically 0.85
/// - `max_iter`: maximum iterations
/// - `tol`: convergence tolerance
///
/// Self-loops are ignored. Returns scores summing to ~1.0.
pub fn compute_pagerank(
    adj: &HashMap<String, Vec<String>>,
    damping: f64,
    max_iter: usize,
    tol: f64,
) -> PageRankResult {
    let nodes: Vec<String> = {
        let mut s: HashSet<String> = adj.keys().cloned().collect();
        for deps in adj.values() {
            for d in deps {
                s.insert(d.clone());
            }
        }
        let mut v: Vec<String> = s.into_iter().collect();
        v.sort();
        v
    };

    let n = nodes.len();
    if n == 0 {
        return PageRankResult {
            scores: HashMap::new(),
            iterations: 0,
        };
    }

    let node_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    // Build adjacency as index lists (excluding self-loops)
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_edges: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (src, deps) in adj {
        let Some(&si) = node_idx.get(src.as_str()) else {
            continue;
        };
        let mut seen = HashSet::new();
        for dep in deps {
            if dep == src {
                continue;
            }
            let Some(&di) = node_idx.get(dep.as_str()) else {
                continue;
            };
            if seen.insert(di) {
                out_edges[si].push(di);
                in_edges[di].push(si);
            }
        }
    }

    let uniform = 1.0 / n as f64;
    let mut scores = vec![uniform; n];
    let base = (1.0 - damping) / n as f64;
    let mut iterations = 0;

    for _ in 0..max_iter {
        iterations += 1;
        let mut new_scores = vec![base; n];

        // Distribute dangling node mass
        let dangling_sum: f64 = (0..n)
            .filter(|&i| out_edges[i].is_empty())
            .map(|i| scores[i])
            .sum();
        let dangling_add = damping * dangling_sum / n as f64;
        for s in &mut new_scores {
            *s += dangling_add;
        }

        // Normal PageRank contribution
        for v in 0..n {
            let out_deg = out_edges[v].len();
            if out_deg == 0 {
                continue;
            }
            let contrib = damping * scores[v] / out_deg as f64;
            for &u in &out_edges[v] {
                new_scores[u] += contrib;
            }
        }

        // Check convergence
        let diff: f64 = scores
            .iter()
            .zip(new_scores.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        scores = new_scores;

        if diff < tol {
            break;
        }
    }

    let mut result = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        result.insert(node.clone(), scores[i]);
    }

    PageRankResult {
        scores: result,
        iterations,
    }
}
