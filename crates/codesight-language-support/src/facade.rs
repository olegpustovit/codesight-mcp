//! Path-indexed façade over `LanguageRegistry`.
//!
//! Downstream crates (agent-readiness, dead-code, hotspot ranking) take a
//! file path and a bit of content, and they do not want to own a
//! `LanguageRegistry` handle themselves. These helpers look up the owning
//! `LanguageSupport` through the process-wide `builtin_registry()` and
//! dispatch each axis.
//!
//! Every helper here replaces a hand-rolled `match path.ends_with(...)`
//! block in `codesight-analysis`.

use std::path::Path;

use crate::builtin_registry;
use crate::manifest::{LanguageId, UNKNOWN_LANGUAGE};
use crate::traits::complexity::{LanguageFamily, NestingMode};
use crate::traits::imports::ImportExtractionMode;
use crate::support::LanguageSupport;

fn lookup_for_path(path: &str) -> Option<&'static dyn LanguageSupport> {
    let id = builtin_registry().identify_path(Path::new(path));
    if id == UNKNOWN_LANGUAGE {
        return None;
    }
    builtin_registry().get(id)
}

/// Canonical language id for a path. Safe default of `UNKNOWN_LANGUAGE`.
pub fn language_id_for_path(path: &str) -> LanguageId {
    builtin_registry().identify_path(Path::new(path))
}

/// Canonical `LanguageFamily` for a path. Drives nesting/function-length
/// heuristics in agent-readiness.
pub fn language_family_for_path(path: &str) -> LanguageFamily {
    lookup_for_path(path)
        .map(|lang| lang.complexity().family())
        .unwrap_or(LanguageFamily::Unknown)
}

/// Brace vs indent nesting for a path.
pub fn nesting_mode_for_path(path: &str) -> NestingMode {
    lookup_for_path(path)
        .map(|lang| lang.complexity().nesting_mode())
        .unwrap_or(NestingMode::Indent)
}

/// Measure type-annotation density for `source`. Returns `None` for
/// languages that don't support measured types or when the signature
/// cannot be parsed.
pub fn measured_type_density_for_path(path: &str, source: &str) -> Option<f64> {
    let lang = lookup_for_path(path)?;
    if !lang.complexity().supports_measured_types() {
        return None;
    }
    lang.complexity().measured_type_density(source)
}

/// Fallback baseline type-density score (used when measurement fails or is
/// unsupported).
pub fn fallback_type_baseline_for_path(path: &str) -> Option<f64> {
    lookup_for_path(path).and_then(|lang| lang.complexity().fallback_type_baseline())
}

/// Function-length warning threshold (lines) for a path.
pub fn function_length_warn_for_path(path: &str) -> Option<u32> {
    lookup_for_path(path).and_then(|lang| lang.complexity().function_length_warn())
}

/// Whether the symbol at `source_text` with `name` is restricted under the
/// language's visibility model. Unknown languages fall through to the
/// permissive default (not restricted).
pub fn has_restricted_visibility(path: &str, source_text: &str, name: &str) -> bool {
    lookup_for_path(path)
        .map(|lang| lang.visibility().is_restricted(source_text, name))
        .unwrap_or(false)
}

/// True when any registered language classifies `path` as a test file.
pub fn is_test_file(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.liveness().is_test_file(path)
    {
        return true;
    }
    // Generic fallbacks that apply regardless of extension (tests/ at the
    // root, /test/ directories).
    let lower = path.to_ascii_lowercase();
    lower.starts_with("tests/")
        || lower.starts_with("test/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/__tests__/")
        || lower.contains("/spec/")
}

/// True when `path` is a fixture/golden/snapshot file for any language.
pub fn is_test_fixture(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.liveness().is_test_fixture(path)
    {
        return true;
    }
    path.contains("/fixtures/")
        || path.starts_with("fixtures/")
        || path.contains("/lang_samples/")
}

/// Entrypoint basenames across every registered language (`main.rs`,
/// `index.ts`, `__main__.py`, …). Flattened so callers that currently scan a
/// single `ENTRYPOINT_BASENAMES` array keep the same shape.
pub fn all_entrypoint_basenames() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = Vec::new();
    for lang in builtin_registry().all() {
        all.extend(lang.liveness().entrypoint_basenames().iter().copied());
    }
    all.sort();
    all.dedup();
    all
}

/// Whether `basename` is an entrypoint for any language.
pub fn is_entrypoint_basename(basename: &str) -> bool {
    for lang in builtin_registry().all() {
        if lang.liveness().entrypoint_basenames().contains(&basename) {
            return true;
        }
    }
    false
}

/// Conventional-path check: returns `true` when any language tags the path
/// as conventional OR when it hits the generic `scripts/`, `tests/`,
/// `examples/`, `benches/`, `fixtures/`, `testdata/`, `stress/`,
/// `build.rs`, `conftest.py` patterns that apply across every language.
pub fn is_conventional_path(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.liveness().is_conventional_path(path)
    {
        return true;
    }
    const GENERIC_DIRS: &[&str] = &[
        "scripts/",
        "tests/",
        "examples/",
        "benches/",
        "fixtures/",
        "testdata/",
        "stress/",
    ];
    if GENERIC_DIRS
        .iter()
        .any(|dir| path.starts_with(dir) || path.contains(&format!("/{dir}")))
    {
        return true;
    }
    let basename = path.rsplit('/').next().unwrap_or(path);
    matches!(basename, "build.rs" | "conftest.py")
}

/// Soft test-weight multiplier for hotspot scoring: test files get 0.3×,
/// production gets 1.0×. Combines the per-language `is_test_file` answer
/// with a generic suffix-based check so legacy scoring stays byte-for-byte.
pub fn test_weight_multiplier(path: &str) -> f64 {
    let basename = path.rsplit('/').next().unwrap_or(path);
    let generic = path.starts_with("tests/")
        || path.contains("/tests/")
        || basename.ends_with("_test.py")
        || basename.ends_with(".test.ts")
        || basename.ends_with(".test.tsx")
        || basename.ends_with(".test.js")
        || basename.ends_with(".test.jsx")
        || basename.ends_with("_spec.rb");
    if generic || is_test_file(path) {
        0.3
    } else {
        1.0
    }
}

fn lookup_for_name(name: &str) -> Option<&'static dyn LanguageSupport> {
    let lower = name.to_ascii_lowercase();
    for lang in builtin_registry().all() {
        let manifest = lang.manifest();
        if lang.id().as_str().eq_ignore_ascii_case(&lower)
            || manifest
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&lower))
        {
            return Some(lang);
        }
    }
    None
}

/// Import-extraction mode for a raw language name (e.g. `"python"`, `"rs"`,
/// `"golang"`, `"tsx"`). Returns `None` for languages whose import extraction
/// is not implemented. Replaces `codesight-imports`'s parallel
/// `IMPORT_EXTRACTION_CAPABILITIES` table (LANGUAGES.md Phase D.5).
pub fn import_extraction_mode_for_language(name: &str) -> Option<ImportExtractionMode> {
    lookup_for_name(name).and_then(|lang| lang.imports().extraction_mode())
}

/// Whether `name` is a language with import-extraction support (tree-sitter
/// or fallback).
pub fn supports_import_extraction(name: &str) -> bool {
    import_extraction_mode_for_language(name).is_some()
}

/// Strip `specifier` to its root package name using the language's package
/// model (e.g. Rust `::`, Python `.`, npm scoped, Go vendor paths).
/// Languages without a package model return the specifier unchanged.
pub fn root_package_from_specifier(name: &str, specifier: &str) -> String {
    match lookup_for_name(name) {
        Some(lang) => lang.packages().root_package_from_specifier(specifier),
        None => specifier.to_string(),
    }
}

/// Ask each registered language for its extra live-root files (foreign-trait
/// impls, `__name__ == '__main__'` files, Cargo targets, …). Flattened and
/// deduplicated.
pub fn additional_live_root_files(
    root: Option<&Path>,
    all_files: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for lang in builtin_registry().all() {
        out.extend(lang.liveness().additional_live_root_files(root, all_files));
    }
    out.sort();
    out.dedup();
    out
}
