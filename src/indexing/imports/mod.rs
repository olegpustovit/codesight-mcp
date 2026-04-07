//! Single subsystem for structural import extraction and resolution.
//!
//! Tree-sitter is the primary extractor for Python, JS/TS/TSX/JSX, and Go when the parse tree
//! has no errors; line-based parsers provide fallback and cover Vue, SQL, and Dart.

mod bundle;
mod cross_repo;
mod line;
mod python_stems;
mod resolve;
mod tree_sitter;

pub use bundle::context_bundle_import_lines;
pub use cross_repo::import_edges_reference_package;
pub use python_stems::python_top_level_import_stems;
pub use resolve::{
    load_tsconfig_aliases, parse_tsconfig_paths, resolve_import_edge_for_graph,
    resolve_imports_to_files, resolve_specifier,
};

use serde::{Deserialize, Serialize};

/// A structured import edge extracted from source code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportEdge {
    pub specifier: String,
    pub names: Vec<String>,
}

/// Detect language from file path extension.
pub fn language_for_path(file_path: &str) -> &'static str {
    let ext = file_path.rsplit('.').next().unwrap_or("");
    match ext {
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "vue" => "vue",
        "sql" => "sql",
        "dart" => "dart",
        "go" => "go",
        "rs" => "rust",
        "rb" => "ruby",
        "php" => "php",
        _ => "unknown",
    }
}

/// Extract structured import edges from source content for any supported language.
pub fn extract_imports(content: &str, file_path: &str, language: &str) -> Vec<ImportEdge> {
    let lang = language.to_ascii_lowercase();
    let path_lang = language_for_path(file_path);
    let effective = if lang == "unknown" && path_lang != "unknown" {
        path_lang.to_string()
    } else {
        lang.clone()
    };

    let edges = match effective.as_str() {
        "python" => match tree_sitter::extract_python_imports_ts(content) {
            Some(v) => v,
            None => line::extract_imports_python(content),
        },
        "javascript" | "typescript" | "tsx" | "jsx" | "js" | "ts" => {
            match tree_sitter::extract_js_imports_ts(content, &effective) {
                Some(v) => v,
                None => line::extract_imports_js(content),
            }
        }
        "vue" => line::extract_imports_vue(content),
        "sql" => line::extract_imports_sql(content),
        "dart" => line::extract_imports_dart(content),
        "go" | "golang" => match tree_sitter::extract_go_imports_ts(content) {
            Some(v) if !v.is_empty() => v,
            _ => line::extract_imports_go(content),
        },
        _ => Vec::new(),
    };

    line::deduplicate_edges(edges, file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_from_import_extracts_module_line_targets() {
        let edges = extract_imports("from b import mid\n", "m.py", "python");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].specifier, "b");
    }

    #[test]
    fn python_import_two_modules() {
        let edges = extract_imports("import os\nimport sys\n", "m.py", "python");
        let specs: Vec<&str> = edges.iter().map(|e| e.specifier.as_str()).collect();
        assert!(specs.contains(&"os"));
        assert!(specs.contains(&"sys"));
    }

    #[test]
    fn python_dotted_from_top_level_resolution_target() {
        let edges = extract_imports("from foo.bar import baz\n", "m.py", "python");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].specifier, "foo.bar");
    }

    #[test]
    fn python_top_level_stems_match_first_segment() {
        let stems = python_top_level_import_stems(
            "from foo.bar import baz\nimport os, sys\n",
            "pkg/mod.py",
        );
        assert!(stems.contains(&"foo".to_string()));
        assert!(stems.contains(&"os".to_string()));
        assert!(stems.contains(&"sys".to_string()));
    }

    #[test]
    fn python_top_level_stems_skip_relative() {
        let stems = python_top_level_import_stems("from . import x\n", "pkg/mod.py");
        assert!(stems.is_empty());
    }
}
