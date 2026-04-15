//! Top-level Python module name stems for repo-wide import heuristics (e.g. "most imported" files).
//! Uses the same structural import extraction as the dependency graph.

use super::extract_imports;

/// For each import in `content`, the first path segment of the module path (e.g. `foo` from `foo.bar`).
/// Relative imports are skipped — they do not match filename-stem to path mapping used by callers.
pub fn python_top_level_import_stems(content: &str, file_path: &str) -> Vec<String> {
    let edges = extract_imports(content, file_path, "python");
    let mut out = Vec::new();
    for edge in edges {
        let s = edge.specifier.trim();
        if s.is_empty() {
            continue;
        }
        if s.starts_with('.') {
            continue;
        }
        if let Some(stem) = s.split('.').next().filter(|x| !x.is_empty()) {
            out.push(stem.to_string());
        }
    }
    out
}
