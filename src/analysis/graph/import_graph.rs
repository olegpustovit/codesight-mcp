//! File-level dependency adjacency from multilingual import extraction.

use std::collections::{HashMap, HashSet};

use crate::core::symbols::FactQuality;
use codesight_imports::imports::{
    extract_imports_with_quality, language_for_path, resolve_import_edge_for_graph,
};

/// Build a file-level dependency adjacency list from source files.
///
/// Uses multilingual import extraction (Python, JS/TS, SQL/dbt, Vue, Dart)
/// and `resolve_specifier` to resolve imports to file paths.
pub fn build_dependency_graph(
    files: &[(String, String)], // (relative_path, source_content)
) -> HashMap<String, Vec<String>> {
    build_dependency_graph_with_aliases(files, None)
}

/// Build a file-level dependency adjacency list from source files,
/// with optional tsconfig/jsconfig path alias support.
pub fn build_dependency_graph_with_aliases(
    files: &[(String, String)], // (relative_path, source_content)
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> HashMap<String, Vec<String>> {
    let mut adj =
        fold_adjacency_from_edges(build_dependency_graph_edges_with_aliases(files, alias_map));
    for (path, _) in files {
        adj.entry(path.clone()).or_default();
    }
    adj
}

/// Resolved file dependency edges with per-source [`FactQuality`] from import extraction.
pub fn build_dependency_graph_edges_with_aliases(
    files: &[(String, String)],
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> Vec<(String, String, FactQuality)> {
    let known_files: HashSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let known_files_vec: Vec<String> = known_files.iter().cloned().collect();
    let mut out: Vec<(String, String, FactQuality)> = Vec::new();

    for (path, content) in files {
        let lang = language_for_path(path);
        let (edges, quality) = extract_imports_with_quality(content, path, lang);
        for edge in &edges {
            if let Some(resolved) = resolve_import_edge_for_graph(
                edge,
                path,
                lang,
                &known_files,
                &known_files_vec,
                alias_map,
            ) && resolved != *path
            {
                out.push((path.clone(), resolved, quality));
            }
        }
    }

    dedupe_preserve_quality(out)
}

fn fold_adjacency_from_edges(
    edges: Vec<(String, String, FactQuality)>,
) -> HashMap<String, Vec<String>> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (src, tgt, _) in edges {
        if seen.insert((src.clone(), tgt.clone())) {
            adj.entry(src).or_default().push(tgt);
        }
    }
    adj
}

/// When duplicate (source, target) pairs appear, keep the weakest (most conservative) quality.
fn dedupe_preserve_quality(
    edges: Vec<(String, String, FactQuality)>,
) -> Vec<(String, String, FactQuality)> {
    use std::collections::BTreeMap;
    let mut best: BTreeMap<(String, String), FactQuality> = BTreeMap::new();
    for (src, tgt, q) in edges {
        let key = (src, tgt);
        let agg = best.entry(key).or_insert(q);
        *agg = weaker_import_fact_quality(*agg, q);
    }
    best.into_iter().map(|((s, t), q)| (s, t, q)).collect()
}

fn weaker_import_fact_quality(a: FactQuality, b: FactQuality) -> FactQuality {
    fn rank(q: FactQuality) -> u8 {
        match q {
            FactQuality::TrustedStructural => 0,
            FactQuality::BaselineStructural => 1,
            FactQuality::Heuristic => 2,
        }
    }
    if rank(a) >= rank(b) { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_edges_carry_structural_quality_for_python_tree_sitter() {
        let files = vec![
            ("a.py".to_string(), "import b\n".to_string()),
            ("b.py".to_string(), String::new()),
        ];
        let edges = build_dependency_graph_edges_with_aliases(&files, None);
        let e = edges
            .iter()
            .find(|(s, t, _)| s == "a.py" && t == "b.py")
            .expect("resolved edge");
        assert_eq!(e.2, FactQuality::BaselineStructural);
    }
}
