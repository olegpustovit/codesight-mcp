//! Import-edge orchestration + generic specifier resolution.
//!
//! Callers hand in `(content, file_path, language)` and get back structured
//! `ImportEdge`s tagged with a coarse [`FactQuality`]. Everything is a thin
//! dispatcher over the per-language `Language` trait — the actual extraction
//! + resolution lives in each `languages/<lang>/imports.rs`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use codesight_core::symbols::FactQuality;

use crate::imports_types::{ImportEdge, deduplicate_edges};
use crate::traits::{ImportExtractionMode, Language};
use crate::{LanguageRegistry, builtin_registry};

// ── Public API ─────────────────────────────────────────────────────────────

/// Lines shown in `assemble_context` / `context_bundle` responses.
/// Dispatches to the language's `display_import_lines_from_source` — Go and
/// Python override; every other language renders via ES-module format.
pub fn context_bundle_import_lines(content: &str, file_path: &str) -> Vec<String> {
    match builtin_registry().for_path(Path::new(file_path)) {
        Some(lang) => lang.display_import_lines_from_source(content),
        None => Vec::new(),
    }
}

pub fn import_extraction_mode(language: &str) -> Option<ImportExtractionMode> {
    crate::facade::import_extraction_mode_for_language(language)
}

pub fn supports_import_extraction(language: &str) -> bool {
    crate::facade::supports_import_extraction(language)
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
    let registry = builtin_registry();
    let path_lang = registry.for_path(Path::new(file_path));
    let named_lang = lookup_by_name(registry, language);
    let lang = named_lang.or(path_lang);

    let Some(lang) = lang else {
        return (Vec::new(), FactQuality::Heuristic);
    };

    let edges = lang.extract_imports(content);
    let edges = deduplicate_edges(edges);
    let ts_used = matches!(
        lang.extraction_mode(),
        Some(ImportExtractionMode::TreeSitterFirst)
    ) && !edges.is_empty();
    let quality = import_fact_quality_for(lang.extraction_mode(), ts_used);
    (edges, quality)
}

fn lookup_by_name<'a>(registry: &'a LanguageRegistry, name: &str) -> Option<&'a dyn Language> {
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
    builtin_registry()
        .identify_path(Path::new(file_path))
        .as_str()
}

/// Extract structured import edges from source content for any supported language.
pub fn extract_imports(content: &str, file_path: &str, language: &str) -> Vec<ImportEdge> {
    extract_imports_with_quality(content, file_path, language).0
}

// ── Generic specifier → file resolution ───────────────────────────────────

/// Resolve an import specifier to a file path within the known source files.
///
/// - `lang`: the owning language — drives the relative-specifier policy via
///   `Language::resolve_relative_specifier`. JS-family languages probe
///   extensions and index files; every other language tries the exact path.
/// - `specifier`: the raw import string (e.g. `"./utils"`, `"react"`).
/// - `from_file`: the file that contains the import.
/// - `source_files`: the set of known file paths in the project.
/// - `alias_map`: optional path-alias mappings (e.g. tsconfig's `@/*`).
pub fn resolve_specifier<L: Language + ?Sized>(
    lang: &L,
    specifier: &str,
    from_file: &str,
    source_files: &HashSet<String>,
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> Option<String> {
    if specifier.starts_with("source:") {
        return None;
    }

    if let Some(aliases) = alias_map
        && let Some(resolved) = resolve_with_aliases(lang, specifier, source_files, aliases)
    {
        return Some(resolved);
    }

    if specifier.starts_with("./") || specifier.starts_with("../") {
        return lang.resolve_relative_specifier(specifier, from_file, source_files);
    }

    if specifier.starts_with('.') {
        return None;
    }

    if source_files.contains(specifier) {
        return Some(specifier.to_string());
    }

    let spec_lower = specifier.to_ascii_lowercase();
    if !specifier.contains('.') && !specifier.contains('/') {
        for file in source_files {
            let file_name = file.rsplit('/').next().unwrap_or(file);
            let stem = file_name
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(file_name);
            if stem.to_ascii_lowercase() == spec_lower {
                return Some(file.clone());
            }
        }
    }

    None
}

fn resolve_with_aliases<L: Language + ?Sized>(
    lang: &L,
    specifier: &str,
    source_files: &HashSet<String>,
    alias_map: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let mut aliases: Vec<(&String, &Vec<String>)> = alias_map.iter().collect();
    aliases.sort_by_key(|a| std::cmp::Reverse(a.0.len()));

    for (pattern, replacements) in aliases {
        let prefix = pattern.trim_end_matches('*');
        if let Some(rest) = specifier.strip_prefix(prefix) {
            for replacement in replacements {
                let repl_prefix = replacement.trim_end_matches('*');
                let resolved_base = format!("{repl_prefix}{rest}");
                let resolved_base = resolved_base.trim_start_matches('/').to_string();
                // Alias resolution routes through the language's relative
                // resolver so extension/index probing is language-specific.
                if let Some(resolved) =
                    lang.resolve_relative_specifier(&resolved_base, "", source_files)
                {
                    return Some(resolved);
                }
                if source_files.contains(&resolved_base) {
                    return Some(resolved_base);
                }
            }
        }
    }
    None
}

/// Resolve a single [`ImportEdge`] to a project file.
///
/// Thin facade over `Language::resolve_import_edge` — picks the language
/// using the explicit `lang` string (falling back to `from_file` when the
/// string is empty / unknown) and dispatches.
///
/// `workspace_crates` is the Rust cross-crate map (`<underscored-crate>` →
/// `<lib.rs path>`); other languages ignore it.
#[allow(clippy::too_many_arguments)]
pub fn resolve_import_edge_for_graph(
    edge: &ImportEdge,
    from_file: &str,
    lang: &str,
    source_files: &HashSet<String>,
    known_files_vec: &[String],
    alias_map: Option<&HashMap<String, Vec<String>>>,
    workspace_crates: Option<&HashMap<String, String>>,
) -> Option<String> {
    let registry = builtin_registry();
    let resolved_lang =
        lookup_by_name(registry, lang).or_else(|| registry.for_path(Path::new(from_file)))?;
    resolved_lang.resolve_import_edge(
        edge,
        from_file,
        source_files,
        known_files_vec,
        alias_map,
        workspace_crates,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
