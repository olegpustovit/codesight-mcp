//! Strongly connected components (Tarjan) for dependency cycle detection.

use std::collections::{HashMap, HashSet};

/// Find all strongly connected components of size > 1 in a directed graph.
/// Returns cycles as sorted Vec<String>.
pub fn find_cycles(adj: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let mut index_counter: usize = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut lowlinks: HashMap<String, usize> = HashMap::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();

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
                    let vl = lowlinks.get_mut(v).unwrap();
                    if wl < *vl {
                        *vl = wl;
                    }
                } else if on_stack.contains(w.as_str()) {
                    let wi = indices[w.as_str()];
                    let vl = lowlinks.get_mut(v).unwrap();
                    if wi < *vl {
                        *vl = wi;
                    }
                }
            }
        }

        if lowlinks[v] == indices[v] {
            let mut scc = Vec::new();
            loop {
                let w = stack.pop().unwrap();
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

    sccs.sort_by(|a, b| a.cmp(b));
    sccs
}
