//! File-level dependency adjacency from multilingual import extraction.

use std::collections::{HashMap, HashSet};

use crate::indexing::imports::{extract_imports, language_for_path, resolve_import_edge_for_graph};

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
    let known_files: HashSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let known_files_vec: Vec<String> = known_files.iter().cloned().collect();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    for (path, content) in files {
        let lang = language_for_path(path);
        let mut deps: Vec<String> = Vec::new();
        let mut seen_deps: HashSet<String> = HashSet::new();

        let edges = extract_imports(content, path, lang);
        for edge in &edges {
            if let Some(resolved) = resolve_import_edge_for_graph(
                edge,
                path,
                lang,
                &known_files,
                &known_files_vec,
                alias_map,
            ) {
                if resolved != *path && seen_deps.insert(resolved.clone()) {
                    deps.push(resolved);
                }
            }
        }

        adj.insert(path.clone(), deps);
    }

    // Ensure all files appear as nodes even if they have no imports
    for (path, _) in files {
        adj.entry(path.clone()).or_default();
    }

    adj
}
