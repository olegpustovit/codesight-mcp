//! Dart language.

pub mod imports;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::DART_MANIFEST;
use crate::manifest::{DART, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::{ImportExtractionMode, Language};
use crate::signature_density::measure_type_before_name;

pub struct Dart;

// Dart keeps a hand-rolled line fallback because the Dart grammar ships
// without a consumer of `import`/`part` directives for dep-graph purposes.

fn load() -> TsLanguage {
    tree_sitter_dart::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "dart",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

fn cfamily_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
    {
        return true;
    }
    !trimmed.starts_with("public ")
}

impl Language for Dart {
    fn id(&self) -> LanguageId {
        DART
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &DART_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::FallbackOnly)
    }
    fn extract_imports(&self, source: &str) -> Vec<ImportEdge> {
        imports::extract(source)
    }
    fn is_restricted(&self, src: &str, name: &str) -> bool {
        cfamily_is_restricted(src, name)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, src: &str) -> Option<f64> {
        measure_type_before_name(src, "dart")
    }
}
