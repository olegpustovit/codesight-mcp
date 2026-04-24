//! File-level dependency adjacency from multilingual import extraction.
//!
//! This is the orchestration layer that turns per-language import extraction
//! (`imports_orchestration::extract_imports_with_quality`) and resolution
//! (`imports_orchestration::resolve_import_edge_for_graph`) into a repo-wide
//! file→file adjacency map with propagated [`FactQuality`].

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

use codesight_core::symbols::FactQuality;

use crate::imports_orchestration::{
    extract_imports_with_quality, language_for_path, resolve_import_edge_for_graph,
};

/// Build a file-level dependency adjacency list from source files.
pub fn build_dependency_graph(files: &[(String, String)]) -> HashMap<String, Vec<String>> {
    build_dependency_graph_with_aliases(files, None, None)
}

/// Build a file-level dependency adjacency list with optional tsconfig/jsconfig
/// path alias support and workspace-crate (Rust) cross-crate `use` resolution.
pub fn build_dependency_graph_with_aliases(
    files: &[(String, String)],
    alias_map: Option<&HashMap<String, Vec<String>>>,
    workspace_crates: Option<&HashMap<String, String>>,
) -> HashMap<String, Vec<String>> {
    let mut adj = fold_adjacency_from_edges(build_dependency_graph_edges_with_aliases(
        files,
        alias_map,
        workspace_crates,
    ));
    for (path, _) in files {
        adj.entry(path.clone()).or_default();
    }
    adj
}

/// Resolved file dependency edges with per-source [`FactQuality`] from import extraction.
///
/// Extraction and per-edge resolution run in parallel across files — for a
/// 500-file Rust repo this is the dominant cost if left sequential, because
/// each file re-parses via tree-sitter to find `use` edges. The shared
/// `known_files` set is immutable during the par-iter, so no locks are needed.
///
/// `files` carries the files whose imports we want to (re)extract — typically
/// the set of files that changed in this ingest. `known_paths`, when `Some`,
/// is the full on-disk file set used for import resolution; when `None`, the
/// set is derived from `files`. Incremental ingests pass `Some` so that edges
/// from new/changed files can still resolve to unchanged files already in the
/// index.
pub fn build_dependency_graph_edges_with_aliases(
    files: &[(String, String)],
    alias_map: Option<&HashMap<String, Vec<String>>>,
    workspace_crates: Option<&HashMap<String, String>>,
) -> Vec<(String, String, FactQuality)> {
    build_dependency_graph_edges_full(files, None, alias_map, workspace_crates)
}

/// Like [`build_dependency_graph_edges_with_aliases`] but accepts an explicit
/// known-path set. Use this on incremental ingests where `files` only carries
/// the changed subset but resolution must see every file on disk.
pub fn build_dependency_graph_edges_full(
    files: &[(String, String)],
    known_paths: Option<&[String]>,
    alias_map: Option<&HashMap<String, Vec<String>>>,
    workspace_crates: Option<&HashMap<String, String>>,
) -> Vec<(String, String, FactQuality)> {
    let known_files: HashSet<String> = match known_paths {
        Some(paths) => paths.iter().cloned().collect(),
        None => files.iter().map(|(p, _)| p.clone()).collect(),
    };
    let known_files_vec: Vec<String> = known_files.iter().cloned().collect();

    let out: Vec<(String, String, FactQuality)> = files
        .par_iter()
        .flat_map(|(path, content)| {
            let lang = language_for_path(path);
            let (edges, quality) = extract_imports_with_quality(content, path, lang);
            let mut resolved: Vec<(String, String, FactQuality)> = Vec::with_capacity(edges.len());
            for edge in &edges {
                if let Some(target) = resolve_import_edge_for_graph(
                    edge,
                    path,
                    lang,
                    &known_files,
                    &known_files_vec,
                    alias_map,
                    workspace_crates,
                ) && target != *path
                {
                    resolved.push((path.clone(), target, quality));
                }
            }
            resolved
        })
        .collect();

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
        let edges = build_dependency_graph_edges_with_aliases(&files, None, None);
        let e = edges
            .iter()
            .find(|(s, t, _)| s == "a.py" && t == "b.py")
            .expect("resolved edge");
        assert_eq!(e.2, FactQuality::BaselineStructural);
    }
}
