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

/// Names the owning language considers idiomatic method / function names
/// that should not be scored as collisions by `symbol_uniqueness`. Returns
/// an empty slice when the path does not resolve to a registered language.
pub fn idiomatic_symbol_names_for_path(path: &str) -> &'static [&'static str] {
    lookup_for_path(path)
        .map(|lang| lang.idiomatic_symbol_names())
        .unwrap_or(&[])
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

/// Outline-noise predicate: `path` is config / manifest / build-script noise
/// for some registered language. The basename set is materialised once on
/// first call (union of every `Language::outline_noise_basenames`) so this
/// runs in O(1) hash lookup on the hot path.
pub fn is_outline_noise_basename(path: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SET.get_or_init(|| {
        builtin_registry()
            .all()
            .flat_map(|lang| lang.outline_noise_basenames().iter().copied())
            .collect()
    });
    let basename = path.rsplit('/').next().unwrap_or(path);
    set.contains(basename)
}

/// Conventional-path check: `true` when the owning language tags the path
/// as conventional, or when any registered language's `conventional_dirs`
/// prefix matches. Language-specific basenames like `build.rs` /
/// `conftest.py` live on the owning language's `is_conventional_path`.
pub fn is_conventional_path(path: &str) -> bool {
    if let Some(lang) = lookup_for_path(path)
        && lang.is_conventional_path(path)
    {
        return true;
    }
    for lang in builtin_registry().all() {
        for dir in lang.conventional_dirs() {
            if path.starts_with(dir) || path.contains(&format!("/{dir}")) {
                return true;
            }
        }
    }
    false
}

/// Soft test-weight multiplier for hotspot scoring: test files get 0.3×,
/// production gets 1.0×. Relies entirely on per-language `is_test_file`.
pub fn test_weight_multiplier(path: &str) -> f64 {
    if is_test_file(path) { 0.3 } else { 1.0 }
}

/// Symbol kinds that distinguish `lang_name` from sibling languages sharing
/// its grammar (see [`Language::distinctive_symbol_kinds`]). Empty when the
/// name doesn't resolve or the language doesn't reclassify.
pub fn distinctive_symbol_kinds_for_language(lang_name: &str) -> &'static [&'static str] {
    lookup_for_name(lang_name)
        .map(|lang| lang.distinctive_symbol_kinds())
        .unwrap_or(&[])
}

/// Post-ingest file-language override. Asks the language named `lang_name`
/// whether `path` (which the indexer labeled as `lang_name` and which emits
/// `has_distinctive_kinds` of its distinctive kinds) should be relabeled.
pub fn reclassify_file_language(
    lang_name: &str,
    path: &str,
    has_distinctive_kinds: bool,
) -> Option<&'static str> {
    lookup_for_name(lang_name)
        .and_then(|lang| lang.reclassify_file_language(path, has_distinctive_kinds))
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
