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
pub fn context_bundle_import_lines(
    content: &str,
    file_path: &str,
    _language: &str,
) -> Vec<String> {
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
    let ts_used = matches!(lang.extraction_mode(), Some(ImportExtractionMode::TreeSitterFirst))
        && !edges.is_empty();
    let quality = import_fact_quality_for(lang.extraction_mode(), ts_used);
    (edges, quality)
}

fn lookup_by_name<'a>(
    registry: &'a LanguageRegistry,
    name: &str,
) -> Option<&'a dyn Language> {
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
/// `specifier`: the raw import string (e.g. `"./utils"`, `"react"`, `"dim_client"`)
/// `from_file`: the file that contains the import
/// `source_files`: set of known file paths in the project
/// `alias_map`: optional path alias mappings (e.g. `{"@/*": ["src/*"]}`)
pub fn resolve_specifier(
    specifier: &str,
    from_file: &str,
    source_files: &HashSet<String>,
    alias_map: Option<&HashMap<String, Vec<String>>>,
) -> Option<String> {
    if specifier.starts_with("source:") {
        return None;
    }

    if let Some(aliases) = alias_map
        && let Some(resolved) = resolve_with_aliases(specifier, source_files, aliases)
    {
        return Some(resolved);
    }

    if specifier.starts_with("./") || specifier.starts_with("../") {
        let dir = parent_dir(from_file);
        let resolved = normalize_path(&format!("{dir}/{specifier}"));
        return resolve_js_path(&resolved, source_files);
    }

    if specifier.starts_with('.') && !specifier.starts_with("./") && !specifier.starts_with("../") {
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

fn resolve_with_aliases(
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
                let resolved_base = resolved_base.trim_start_matches('/');
                if let Some(resolved) = resolve_js_path(resolved_base, source_files) {
                    return Some(resolved);
                }
            }
        }
    }
    None
}

fn resolve_js_path(base: &str, source_files: &HashSet<String>) -> Option<String> {
    if source_files.contains(base) {
        return Some(base.to_string());
    }

    let extensions = [".ts", ".tsx", ".js", ".jsx", ".vue", ".mjs", ".cjs"];
    for ext in &extensions {
        let with_ext = format!("{base}{ext}");
        if source_files.contains(&with_ext) {
            return Some(with_ext);
        }
    }

    let index_extensions = ["index.ts", "index.tsx", "index.js", "index.jsx"];
    for idx in &index_extensions {
        let with_index = format!("{base}/{idx}");
        if source_files.contains(&with_index) {
            return Some(with_index);
        }
    }

    if let Some(without) = base.strip_suffix(".js") {
        let ts_extensions = [".ts", ".tsx"];
        for ext in &ts_extensions {
            let mapped = format!("{without}{ext}");
            if source_files.contains(&mapped) {
                return Some(mapped);
            }
        }
    }

    None
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => ".".to_string(),
    }
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Load path aliases from tsconfig.json (supports JSONC with comments).
pub fn load_tsconfig_aliases(source_root: &str) -> HashMap<String, Vec<String>> {
    if source_root.is_empty() {
        return HashMap::new();
    }

    let tsconfig_path = format!("{source_root}/tsconfig.json");
    let content = match std::fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    parse_tsconfig_paths(&content)
}

/// Parse tsconfig JSON(C) content and extract compilerOptions.paths.
pub fn parse_tsconfig_paths(content: &str) -> HashMap<String, Vec<String>> {
    let clean = strip_jsonc_comments(content);
    let parsed: serde_json::Value = match serde_json::from_str(&clean) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let paths = match parsed.get("compilerOptions").and_then(|co| co.get("paths")) {
        Some(p) => p,
        None => return HashMap::new(),
    };

    let mut result = HashMap::new();
    if let Some(obj) = paths.as_object() {
        for (key, value) in obj {
            if let Some(arr) = value.as_array() {
                let normalized: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| {
                        if s == "./*" {
                            "/*".to_string()
                        } else {
                            s.strip_prefix("./").unwrap_or(s).to_string()
                        }
                    })
                    .collect();
                result.insert(key.clone(), normalized);
            }
        }
    }
    result
}

fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;

    while i < len {
        if in_string {
            result.push(chars[i]);
            if chars[i] == '\\' && i + 1 < len {
                i += 1;
                result.push(chars[i]);
            } else if chars[i] == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            result.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '/' && i + 1 < len {
            if chars[i + 1] == '/' {
                i += 2;
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            } else if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Resolve import targets to known file paths (Python stem → `foo.py`, package `__init__.py`, etc.).
pub fn resolve_imports_to_files(
    import_targets: &[String],
    known_files: &[String],
    source_file: &str,
) -> Vec<String> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    let mut stem_to_file: HashMap<String, Vec<String>> = HashMap::new();
    for f in known_files {
        if let Some(stem) = file_stem(f) {
            stem_to_file.entry(stem).or_default().push(f.clone());
        }
    }

    for target in import_targets {
        if let Some(matches) = stem_to_file.get(target.as_str()) {
            for m in matches {
                if m != source_file && seen.insert(m.clone()) {
                    resolved.push(m.clone());
                }
            }
        }
    }

    resolved
}

fn file_stem(path: &str) -> Option<String> {
    let file_name = path.rsplit('/').next()?;
    let stem = file_name
        .strip_suffix(".py")
        .or_else(|| file_name.strip_suffix(".js"))
        .or_else(|| file_name.strip_suffix(".ts"))
        .or_else(|| file_name.strip_suffix(".go"))
        .or_else(|| file_name.strip_suffix(".rs"))
        .or_else(|| file_name.strip_suffix(".rb"))
        .unwrap_or(file_name);
    if stem == "__init__" {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some(parts[parts.len() - 2].to_string());
        }
    }
    Some(stem.to_string())
}

/// Resolve a single [`ImportEdge`] to a project file.
///
/// This is a thin facade over `Language::resolve_import_edge` — it picks the
/// language using the explicit `lang` string (falling back to `from_file`
/// when the string is empty / unknown) and dispatches.
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
    let resolved_lang = lookup_by_name(registry, lang)
        .or_else(|| registry.for_path(Path::new(from_file)))?;
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
