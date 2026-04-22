//! Path-indexed façade over `LanguageRegistry`.
//!
//! Downstream crates (agent-readiness, dead-code, hotspot ranking) take a
//! file path and a bit of content, and they do not want to own a
//! `LanguageRegistry` handle themselves. These helpers look up the owning
//! `Language` through the process-wide `builtin_registry()` and dispatch
//! directly through the trait.

use std::path::Path;

use crate::builtin_registry;
use crate::family::{LanguageFamily, NestingMode};
use crate::fallback::{generic_is_test_file, generic_is_test_fixture};
use crate::manifest::{LanguageId, UNKNOWN_LANGUAGE};
use crate::traits::{ImportExtractionMode, Language};

fn lookup_for_path(path: &str) -> Option<&'static dyn Language> {
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

/// Canonical `LanguageFamily` for a path.
pub fn language_family_for_path(path: &str) -> LanguageFamily {
    lookup_for_path(path)
        .map(|lang| lang.family())
        .unwrap_or(LanguageFamily::Unknown)
}

/// Brace vs indent nesting for a path.
pub fn nesting_mode_for_path(path: &str) -> NestingMode {
    lookup_for_path(path)
        .map(|lang| lang.nesting_mode())
        .unwrap_or(NestingMode::Indent)
}

/// Measure type-annotation density for `source`.
pub fn measured_type_density_for_path(path: &str, source: &str) -> Option<f64> {
    let lang = lookup_for_path(path)?;
    if !lang.supports_measured_types() {
        return None;
    }
    lang.measured_type_density(source)
}

/// Fallback baseline type-density score.
pub fn fallback_type_baseline_for_path(path: &str) -> Option<f64> {
    lookup_for_path(path).and_then(|lang| lang.fallback_type_baseline())
}

/// Function-length warning threshold (lines) for a path.
pub fn function_length_warn_for_path(path: &str) -> Option<u32> {
    lookup_for_path(path).and_then(|lang| lang.function_length_warn())
}

/// Whether the symbol at `source_text` with `name` is restricted.
pub fn has_restricted_visibility(path: &str, source_text: &str, name: &str) -> bool {
    lookup_for_path(path)
        .map(|lang| lang.is_restricted(source_text, name))
        .unwrap_or(false)
}

/// Class-hierarchy advisory note for `path`, or `None` when the language
/// either models class inheritance normally (C++/Java/Python) or is not
/// registered.
pub fn class_hierarchy_note_for_path(path: &str) -> Option<&'static str> {
    lookup_for_path(path).and_then(|lang| lang.class_hierarchy_note())
}

/// Whole-word match for `name` in `source`, using the owning language's
/// identifier-boundary rules when `path` resolves to a registered
/// language; falls back to the ASCII default otherwise.
pub fn mentions_identifier_in_file(path: &str, source: &str, name: &str) -> bool {
    match lookup_for_path(path) {
        Some(lang) => lang.mentions_identifier(source, name),
        None => crate::text_match::word_match(source, name),
    }
}

/// Case-insensitive variant of [`mentions_identifier_in_file`].
pub fn mentions_identifier_ci_in_file(path: &str, source: &str, name: &str) -> bool {
    match lookup_for_path(path) {
        Some(lang) => lang.mentions_identifier_ignore_case(source, name),
        None => crate::text_match::ci_word_match(source, name),
    }
}

/// True when any registered language classifies `path` as a test file.
pub fn is_test_file(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.is_test_file(path)
    {
        return true;
    }
    generic_is_test_file(path)
}

/// True when `path` is a fixture/golden/snapshot file for any language.
pub fn is_test_fixture(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.is_test_fixture(path)
    {
        return true;
    }
    generic_is_test_fixture(path)
}

/// Entrypoint basenames across every registered language. Flattened.
pub fn all_entrypoint_basenames() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = Vec::new();
    for lang in builtin_registry().all() {
        all.extend(lang.entrypoint_basenames().iter().copied());
    }
    all.sort();
    all.dedup();
    all
}

/// Whether `basename` is an entrypoint for any language.
pub fn is_entrypoint_basename(basename: &str) -> bool {
    for lang in builtin_registry().all() {
        if lang.entrypoint_basenames().contains(&basename) {
            return true;
        }
    }
    false
}

/// Conventional-path check: returns `true` when any language tags the path
/// as conventional OR when it hits the generic non-production directory set.
pub fn is_conventional_path(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.is_conventional_path(path)
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
/// production gets 1.0×.
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

fn lookup_for_name(name: &str) -> Option<&'static dyn Language> {
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

/// Import-extraction mode for a raw language name.
pub fn import_extraction_mode_for_language(name: &str) -> Option<ImportExtractionMode> {
    lookup_for_name(name).and_then(|lang| lang.extraction_mode())
}

/// Whether `name` is a language with import-extraction support.
pub fn supports_import_extraction(name: &str) -> bool {
    import_extraction_mode_for_language(name).is_some()
}

/// Strip `specifier` to its root package name using the language's model.
pub fn root_package_from_specifier(name: &str, specifier: &str) -> String {
    match lookup_for_name(name) {
        Some(lang) => lang.root_package_from_specifier(specifier),
        None => specifier.to_string(),
    }
}

/// Ask each registered language for its extra live-roots. Returns the
/// structured [`LiveRoot`] list so callers can attach per-reason evidence
/// notes without branching on the emitting language.
pub fn additional_live_roots(
    root: Option<&Path>,
    all_files: &[String],
) -> Vec<crate::traits::LiveRoot> {
    builtin_registry()
        .all()
        .flat_map(|lang| lang.additional_live_roots(root, all_files))
        .collect()
}
