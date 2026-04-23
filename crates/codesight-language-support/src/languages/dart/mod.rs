//! Dart language.

pub mod imports;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::DART_MANIFEST;
use crate::manifest::{DART, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::{ImportExtractionMode, Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

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
    fn is_restricted(&self, src: &str, _name: &str) -> bool {
        cfamily_is_restricted(src)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn type_annotation_style(&self) -> TypeAnnotationStyle {
        TypeAnnotationStyle::TypeBeforeName
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["dynamic"]
    }
}
