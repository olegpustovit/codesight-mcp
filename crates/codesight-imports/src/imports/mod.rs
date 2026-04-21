//! Import-edge orchestration.
//!
//! Per-language extractors now live in `codesight-language-support`. This
//! module is a thin dispatcher: resolve the language from the path, call
//! `Language::extract_imports`, attach a [`FactQuality`] tag for storage.

mod cross_repo;
mod resolve;

pub use codesight_language_support::{ImportEdge, ImportExtractionMode};

/// Lines shown in `assemble_context` / `context_bundle` responses.
/// Dispatches to the language's `display_import_lines_from_source` — Go and
/// Python override; every other language renders via ES-module format.
pub fn context_bundle_import_lines(
    content: &str,
    file_path: &str,
    _language: &str,
) -> Vec<String> {
    let registry = codesight_language_support::builtin_registry();
    match registry.for_path(std::path::Path::new(file_path)) {
        Some(lang) => lang.display_import_lines_from_source(content),
        None => Vec::new(),
    }
}
pub use cross_repo::{
    PackageMatchKind, PackageReferenceEvidence, PackageReferenceMatcher, package_reference_evidence,
};
pub use resolve::{
    load_tsconfig_aliases, parse_tsconfig_paths, resolve_import_edge_for_graph,
    resolve_imports_to_files, resolve_specifier,
};

use codesight_core::symbols::FactQuality;

pub fn import_extraction_mode(language: &str) -> Option<ImportExtractionMode> {
    codesight_language_support::import_extraction_mode_for_language(language)
}

pub fn supports_import_extraction(language: &str) -> bool {
    codesight_language_support::supports_import_extraction(language)
}

#[must_use]
fn import_fact_quality_for(
    mode: Option<ImportExtractionMode>,
    tree_sitter_used: bool,
) -> FactQuality {
    match mode {
        Some(ImportExtractionMode::FallbackOnly) => FactQuality::Heuristic,
        Some(_) if tree_sitter_used => FactQuality::BaselineStructural,
        Some(_) => FactQuality::Heuristic,
        None => FactQuality::Heuristic,
    }
}

/// Extract structured import edges together with coarse quality for
/// dependency-graph storage.
#[must_use]
pub fn extract_imports_with_quality(
    content: &str,
    file_path: &str,
    language: &str,
) -> (Vec<ImportEdge>, FactQuality) {
    let registry = codesight_language_support::builtin_registry();
    let path_lang = registry.for_path(std::path::Path::new(file_path));
    let named_lang = lookup_by_name(registry, language);
    let lang = named_lang.or(path_lang);

    let Some(lang) = lang else {
        return (Vec::new(), FactQuality::Heuristic);
    };

    let edges = lang.extract_imports(content);
    let edges = codesight_language_support::imports_types::deduplicate_edges(edges);
    // Coarse quality: TreeSitterFirst + non-empty result → structural,
    // otherwise heuristic. We don't know here whether ts or fallback ran;
    // approximate by `TreeSitterFirst && !empty`.
    let ts_used = matches!(lang.extraction_mode(), Some(ImportExtractionMode::TreeSitterFirst))
        && !edges.is_empty();
    let quality = import_fact_quality_for(lang.extraction_mode(), ts_used);
    (edges, quality)
}

fn lookup_by_name(
    registry: &'static codesight_language_support::LanguageRegistry,
    name: &str,
) -> Option<&'static dyn codesight_language_support::Language> {
    if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
        return None;
    }
    registry.all().find(|lang| {
        let id = lang.id().as_str();
        if id.eq_ignore_ascii_case(name) {
            return true;
        }
        lang.manifest()
            .aliases
            .iter()
            .any(|a| a.eq_ignore_ascii_case(name))
    })
}

/// Detect language id string from a file path.
pub fn language_for_path(file_path: &str) -> &'static str {
    codesight_language_support::builtin_registry()
        .identify_path(std::path::Path::new(file_path))
        .as_str()
}

/// Extract structured import edges from source content for any supported language.
pub fn extract_imports(content: &str, file_path: &str, language: &str) -> Vec<ImportEdge> {
    extract_imports_with_quality(content, file_path, language).0
}

/// Top-level Python module stems (kept as a free fn for the existing callers
/// in storage that need per-file Python stem extraction without a registry
/// round-trip). Delegates to the Python lang impl.
pub fn python_top_level_import_stems(content: &str, file_path: &str) -> Vec<String> {
    let registry = codesight_language_support::builtin_registry();
    let Some(python) = registry.get(codesight_language_support::PYTHON) else {
        return Vec::new();
    };
    python.top_level_import_stems(content, file_path)
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
