//! Single subsystem for structural import extraction and resolution.
//!
//! Tree-sitter is the primary extractor for Python, JS/TS/TSX/JSX, Go, and Rust when the
//! parse tree has no errors; line-based parsers provide fallback and cover Vue, SQL, and Dart.

mod bundle;
mod cross_repo;
mod fallback;
mod python_stems;
mod resolve;
mod tree_sitter;

pub use bundle::context_bundle_import_lines;
pub use codesight_language_support::ImportExtractionMode;
pub use cross_repo::{
    PackageMatchKind, PackageReferenceEvidence, PackageReferenceMatcher, package_reference_evidence,
};
pub use python_stems::python_top_level_import_stems;
pub use resolve::{
    load_tsconfig_aliases, parse_tsconfig_paths, resolve_import_edge_for_graph,
    resolve_imports_to_files, resolve_specifier,
};

use serde::{Deserialize, Serialize};

use codesight_core::symbols::FactQuality;

/// A structured import edge extracted from source code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportEdge {
    pub specifier: String,
    pub names: Vec<String>,
}

pub fn import_extraction_mode(language: &str) -> Option<ImportExtractionMode> {
    codesight_language_support::import_extraction_mode_for_language(language)
}

pub fn supports_import_extraction(language: &str) -> bool {
    codesight_language_support::supports_import_extraction(language)
}

#[must_use]
fn import_fact_quality_for(language: &str, tree_sitter_used: bool) -> FactQuality {
    match import_extraction_mode(language) {
        Some(ImportExtractionMode::FallbackOnly) => FactQuality::Heuristic,
        Some(_) if tree_sitter_used => FactQuality::BaselineStructural,
        Some(_) => FactQuality::Heuristic,
        None => FactQuality::Heuristic,
    }
}

/// Extract structured import edges together with coarse quality for dependency-graph storage.
///
/// Tree-sitter-backed paths for supported languages map to [`FactQuality::BaselineStructural`];
/// line fallbacks and `FallbackOnly` languages map to [`FactQuality::Heuristic`].
#[must_use]
pub fn extract_imports_with_quality(
    content: &str,
    file_path: &str,
    language: &str,
) -> (Vec<ImportEdge>, FactQuality) {
    let lang = language.to_ascii_lowercase();
    let path_lang = language_for_path(file_path);
    let effective = if lang == "unknown" && path_lang != "unknown" {
        path_lang.to_string()
    } else {
        lang.clone()
    };

    let (edges, used_tree_sitter) = match effective.as_str() {
        "python" => match tree_sitter::extract_python_imports_ts(content) {
            Some(v) => (v, true),
            None => (fallback::python::extract_imports(content), false),
        },
        "javascript" | "typescript" | "tsx" | "jsx" | "js" | "ts" => {
            match tree_sitter::extract_js_imports_ts(content, &effective) {
                Some(v) => (v, true),
                None => (fallback::js_ts::extract_imports(content), false),
            }
        }
        "vue" => (fallback::vue::extract_imports(content), false),
        "sql" => (fallback::sql::extract_imports(content), false),
        "dart" => (fallback::dart::extract_imports(content), false),
        "go" | "golang" => match tree_sitter::extract_go_imports_ts(content) {
            Some(v) if !v.is_empty() => (v, true),
            _ => (fallback::go::extract_imports(content), false),
        },
        "rust" | "rs" => match tree_sitter::extract_rust_imports_ts(content) {
            Some(v) => (v, true),
            None => (fallback::rust::extract_imports(content), false),
        },
        _ => (Vec::new(), false),
    };

    let edges = fallback::helpers::deduplicate_edges(edges);
    let quality = import_fact_quality_for(&effective, used_tree_sitter);
    (edges, quality)
}

/// Detect language from file path extension. Thin adapter over
/// [`codesight_language_support::LanguageRegistry`] — the registry owns
/// the full identity table (LANGUAGES.md §5 `language_for_path` migration).
pub fn language_for_path(file_path: &str) -> &'static str {
    codesight_language_support::builtin_registry()
        .identify_path(std::path::Path::new(file_path))
        .as_str()
}

/// Extract structured import edges from source content for any supported language.
pub fn extract_imports(content: &str, file_path: &str, language: &str) -> Vec<ImportEdge> {
    extract_imports_with_quality(content, file_path, language).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use codesight_core::symbols::FactQuality;

    #[test]
    fn python_imports_with_quality_is_structural_when_tree_sitter_runs() {
        let (_edges, q) = extract_imports_with_quality("from b import mid\n", "m.py", "python");
        assert_eq!(q, FactQuality::BaselineStructural);
    }

    #[test]
    fn dart_imports_with_quality_is_heuristic_fallback_only() {
        let (_edges, q) = extract_imports_with_quality("import 'dart:io';\n", "main.dart", "dart");
        assert_eq!(q, FactQuality::Heuristic);
    }

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

    #[test]
    fn import_capabilities_cover_dart_and_go() {
        assert_eq!(
            import_extraction_mode("dart"),
            Some(ImportExtractionMode::FallbackOnly)
        );
        assert_eq!(
            import_extraction_mode("golang"),
            Some(ImportExtractionMode::TreeSitterFirst)
        );
        assert_eq!(
            import_extraction_mode("go"),
            Some(ImportExtractionMode::TreeSitterFirst)
        );
    }
}
