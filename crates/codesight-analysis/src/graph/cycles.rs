//! Strongly connected components (Tarjan, via `petgraph`) for dependency
//! cycle detection.

use std::collections::{HashMap, HashSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

/// Edges that represent module containment ("foo/bar.rs uses its parent
/// mod.rs" or "foo.rs uses the crate root lib.rs") are structural back-edges,
/// not runtime dependencies. Including them in cycle detection inflates
/// strongly connected components with noise (one 20+ file "cycle" across
/// siblings that all re-export through a shared `mod.rs`).
///
/// An edge `src → tgt` is treated as a structural back-edge when `tgt` is
/// an ancestor module file of `src`: either `<some-prefix>/mod.rs`,
/// `<some-prefix>/lib.rs`, or `<some-prefix>/main.rs` where the prefix
/// is a directory ancestor of `src`.
fn is_structural_back_edge(src: &str, tgt: &str) -> bool {
    let Some((tgt_dir, tgt_file)) = tgt.rsplit_once('/') else {
        // Crate-root at repo root (no directory).
        return matches!(tgt, "mod.rs" | "lib.rs" | "main.rs");
    };
    if !matches!(tgt_file, "mod.rs" | "lib.rs" | "main.rs") {
        return false;
    }
    src != tgt
        && (src.starts_with(&format!("{tgt_dir}/"))
            || src == tgt_dir
            || (tgt_dir.is_empty() && src.contains('/')))
}

/// Find all strongly connected components of size > 1 in a directed graph.
/// Returns cycles as sorted Vec<String>. Structural back-edges from child
/// modules to their ancestor `mod.rs` / `lib.rs` / `main.rs` are excluded
/// from the SCC search so they do not inflate extractor-tree cycles.
pub fn find_cycles(adj: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let mut nodes: Vec<&str> = adj.keys().map(String::as_str).collect();
    nodes.sort();

    let mut graph: DiGraph<&str, ()> = DiGraph::new();
    let mut idx: HashMap<&str, _> = HashMap::with_capacity(nodes.len());
    for node in &nodes {
        idx.insert(*node, graph.add_node(*node));
    }
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for src in &nodes {
        let Some(targets) = adj.get(*src) else {
            continue;
        };
        for tgt in targets {
            let tgt_str = tgt.as_str();
            if src == &tgt_str || is_structural_back_edge(src, tgt_str) {
                continue;
            }
            let Some(&tgt_ix) = idx.get(tgt_str) else {
                continue;
            };
            if seen.insert((src, tgt_str)) {
                graph.add_edge(idx[src], tgt_ix, ());
            }
        }
    }

    let mut sccs: Vec<Vec<String>> = tarjan_scc(&graph)
        .into_iter()
        .filter(|scc| scc.len() > 1)
        .map(|scc| {
            let mut names: Vec<String> = scc.into_iter().map(|ix| graph[ix].to_string()).collect();
            names.sort();
            names
        })
        .collect();
    sccs.sort();
    sccs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adj(edges: &[(&str, &str)]) -> HashMap<String, Vec<String>> {
        let mut a: HashMap<String, Vec<String>> = HashMap::new();
        for (s, t) in edges {
            a.entry((*s).to_string())
                .or_default()
                .push((*t).to_string());
            a.entry((*t).to_string()).or_default();
        }
        a
    }

    #[test]
    fn structural_back_edges_dont_create_cycles() {
        let a = adj(&[
            ("src/extractors/mod.rs", "src/extractors/asm.rs"),
            ("src/extractors/asm.rs", "src/extractors/mod.rs"),
        ]);
        assert!(find_cycles(&a).is_empty());
    }

    #[test]
    fn real_cycle_between_siblings_is_reported() {
        let a = adj(&[
            ("src/a/foo.rs", "src/a/bar.rs"),
            ("src/a/bar.rs", "src/a/foo.rs"),
        ]);
        let cycles = find_cycles(&a);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0], vec!["src/a/bar.rs", "src/a/foo.rs"]);
    }

    #[test]
    fn child_to_crate_root_lib_rs_is_structural() {
        let a = adj(&[
            ("src/lib.rs", "src/service.rs"),
            ("src/service.rs", "src/lib.rs"),
        ]);
        assert!(find_cycles(&a).is_empty());
    }
}
