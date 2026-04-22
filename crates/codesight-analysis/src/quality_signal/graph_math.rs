//! Graph-shaped math backing the structural signal: boundary cohesion,
//! condensed dependency depth, longest-path-in-DAG, and the Gini coefficient.

use std::collections::HashMap;

fn top_level_module(path: &str) -> &str {
    path.split('/').next().unwrap_or(".")
}

/// Boundary-cohesion: share of dependency edges that stay within the same
/// top-level directory. Returns `None` when the graph has no usable edges.
pub fn compute_module_cohesion(adj: &HashMap<String, Vec<String>>) -> Option<f64> {
    let mut total = 0usize;
    let mut internal = 0usize;
    for (src, targets) in adj {
        for tgt in targets {
            if src == tgt {
                continue;
            }
            total += 1;
            if top_level_module(src) == top_level_module(tgt) {
                internal += 1;
            }
        }
    }
    (total > 0).then_some(internal as f64 / total as f64)
}

/// Compute layering depth on the SCC-condensed DAG.
pub fn condensed_dependency_depth(
    adj: &HashMap<String, Vec<String>>,
    cycles: &[Vec<String>],
) -> Option<usize> {
    if adj.is_empty() {
        return None;
    }

    let mut node_to_scc: HashMap<&str, usize> = HashMap::new();
    let mut scc_count = 0usize;

    for scc in cycles {
        for node in scc {
            node_to_scc.insert(node.as_str(), scc_count);
        }
        scc_count += 1;
    }
    for node in adj.keys() {
        if !node_to_scc.contains_key(node.as_str()) {
            node_to_scc.insert(node.as_str(), scc_count);
            scc_count += 1;
        }
    }

    let mut dag: HashMap<usize, Vec<usize>> = HashMap::new();
    for (src, targets) in adj {
        let src_scc = node_to_scc[src.as_str()];
        for tgt in targets {
            if let Some(&tgt_scc) = node_to_scc.get(tgt.as_str())
                && src_scc != tgt_scc
            {
                dag.entry(src_scc).or_default().push(tgt_scc);
            }
        }
    }

    longest_path_in_dag(&dag, scc_count)
}

/// Longest path in a DAG using Kahn's algorithm + DP.
fn longest_path_in_dag(dag: &HashMap<usize, Vec<usize>>, node_count: usize) -> Option<usize> {
    let mut in_degree = vec![0usize; node_count];
    for targets in dag.values() {
        for &t in targets {
            if t < node_count {
                in_degree[t] += 1;
            }
        }
    }

    let mut queue: std::collections::VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|&(_, d)| *d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut dist = vec![0usize; node_count];
    let mut max_depth = 0usize;

    while let Some(u) = queue.pop_front() {
        if let Some(targets) = dag.get(&u) {
            for &v in targets {
                if v >= node_count {
                    continue;
                }
                let new_dist = dist[u] + 1;
                if new_dist > dist[v] {
                    dist[v] = new_dist;
                    max_depth = max_depth.max(new_dist);
                }
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }
    }

    Some(max_depth)
}

/// Gini coefficient over a distribution of values.
/// Returns 0.0 (perfect equality) to 1.0 (maximum inequality).
pub fn gini_coefficient(values: &[f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f64 = sorted.iter().sum();
    if sum == 0.0 {
        return 0.0;
    }
    let mut cumulative = 0.0;
    let mut area_under_lorenz = 0.0;
    for &v in &sorted {
        cumulative += v;
        area_under_lorenz += cumulative;
    }
    let gini = 1.0 - (2.0 * area_under_lorenz) / (n as f64 * sum) + 1.0 / n as f64;
    gini.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::find_cycles;

    #[test]
    fn gini_uniform_is_zero() {
        let vals = vec![5.0, 5.0, 5.0, 5.0];
        let g = gini_coefficient(&vals);
        assert!(g < 0.01, "uniform distribution should have Gini ~0, got {g}");
    }

    #[test]
    fn gini_skewed_is_high() {
        let vals = vec![0.0, 0.0, 0.0, 100.0];
        let g = gini_coefficient(&vals);
        assert!(g > 0.5, "skewed distribution should have high Gini, got {g}");
    }

    #[test]
    fn gini_empty_is_zero() {
        assert_eq!(gini_coefficient(&[]), 0.0);
    }

    #[test]
    fn top_level_module_extracts_first_segment() {
        assert_eq!(top_level_module("src/lib.rs"), "src");
        assert_eq!(top_level_module("crates/core/src/lib.rs"), "crates");
        assert_eq!(top_level_module("lib.rs"), "lib.rs");
    }

    #[test]
    fn cohesion_all_internal() {
        let adj = HashMap::from([
            ("src/a.rs".to_string(), vec!["src/b.rs".to_string()]),
            ("src/b.rs".to_string(), vec!["src/c.rs".to_string()]),
        ]);
        let c = compute_module_cohesion(&adj).unwrap();
        assert!((c - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cohesion_all_external() {
        let adj = HashMap::from([
            ("src/a.rs".to_string(), vec!["tests/b.rs".to_string()]),
            ("tests/b.rs".to_string(), vec!["src/c.rs".to_string()]),
        ]);
        let c = compute_module_cohesion(&adj).unwrap();
        assert!(c < 0.01);
    }

    #[test]
    fn cohesion_empty_graph() {
        let adj: HashMap<String, Vec<String>> = HashMap::new();
        assert!(compute_module_cohesion(&adj).is_none());
    }

    #[test]
    fn depth_flat_graph() {
        let adj = HashMap::from([
            ("a".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec![]),
        ]);
        let cycles = find_cycles(&adj);
        let d = condensed_dependency_depth(&adj, &cycles).unwrap();
        assert_eq!(d, 1);
    }

    #[test]
    fn depth_empty_graph() {
        let adj: HashMap<String, Vec<String>> = HashMap::new();
        assert!(condensed_dependency_depth(&adj, &[]).is_none());
    }
}
