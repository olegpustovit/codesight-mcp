//! Strongly connected components (Tarjan) for dependency cycle detection.

use std::collections::{HashMap, HashSet};

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
    // For `lib.rs` / `main.rs` we treat the target dir itself as the module
    // root. For `mod.rs` the module root is the directory the file sits in.
    // In both cases, `src` being inside `tgt_dir/` (or equal to `tgt_dir`)
    // means the edge points at a strictly-ancestor module file.
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
    let mut index_counter: usize = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut lowlinks: HashMap<String, usize> = HashMap::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn strongconnect(
        v: &str,
        adj: &HashMap<String, Vec<String>>,
        index_counter: &mut usize,
        stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        indices: &mut HashMap<String, usize>,
        lowlinks: &mut HashMap<String, usize>,
        sccs: &mut Vec<Vec<String>>,
    ) {
        indices.insert(v.to_string(), *index_counter);
        lowlinks.insert(v.to_string(), *index_counter);
        *index_counter += 1;
        stack.push(v.to_string());
        on_stack.insert(v.to_string());

        if let Some(neighbors) = adj.get(v) {
            for w in neighbors {
                if w == v {
                    // Skip self-loops
                    continue;
                }
                if is_structural_back_edge(v, w) {
                    continue;
                }
                if !indices.contains_key(w.as_str()) {
                    strongconnect(
                        w,
                        adj,
                        index_counter,
                        stack,
                        on_stack,
                        indices,
                        lowlinks,
                        sccs,
                    );
                    let wl = lowlinks[w.as_str()];
                    // Invariant: `v` gained a lowlink when this frame started.
                    let vl = lowlinks.get_mut(v).expect("lowlink for current vertex");
                    if wl < *vl {
                        *vl = wl;
                    }
                } else if on_stack.contains(w.as_str()) {
                    let wi = indices[w.as_str()];
                    let vl = lowlinks.get_mut(v).expect("lowlink for current vertex");
                    if wi < *vl {
                        *vl = wi;
                    }
                }
            }
        }

        if lowlinks[v] == indices[v] {
            let mut scc = Vec::new();
            loop {
                // Invariant: SCC head `v` remains on the stack until popped here.
                let w = stack.pop().expect("non-empty stack closing SCC");
                on_stack.remove(&w);
                scc.push(w.clone());
                if w == v {
                    break;
                }
            }
            if scc.len() > 1 {
                scc.sort();
                sccs.push(scc);
            }
        }
    }

    // Visit all nodes to handle disconnected components
    let mut all_nodes: Vec<String> = adj.keys().cloned().collect();
    all_nodes.sort(); // deterministic order
    for node in &all_nodes {
        if !indices.contains_key(node.as_str()) {
            strongconnect(
                node,
                adj,
                &mut index_counter,
                &mut stack,
                &mut on_stack,
                &mut indices,
                &mut lowlinks,
                &mut sccs,
            );
        }
    }

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
        // `asm.rs` re-exports through `extractors/mod.rs`, which `pub mod asm;`s
        // it back. That child→parent edge is structural and should not make
        // the pair look like a runtime cycle.
        let a = adj(&[
            ("src/extractors/mod.rs", "src/extractors/asm.rs"),
            ("src/extractors/asm.rs", "src/extractors/mod.rs"),
        ]);
        assert!(find_cycles(&a).is_empty());
    }

    #[test]
    fn real_cycle_between_siblings_is_reported() {
        // Two siblings that genuinely cross-reference each other with
        // no parent-mod indirection should still show up.
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
