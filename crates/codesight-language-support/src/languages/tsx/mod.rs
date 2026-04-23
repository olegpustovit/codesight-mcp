//! TSX language.

use std::collections::HashSet;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::TSX_MANIFEST;
use crate::languages::javascript::{
    JS_CONVENTIONAL_DIRS, JS_ENTRYPOINTS, JS_TEST_FILE_DIRS, JS_TEST_FILE_SUFFIXES,
    JS_TEST_FIXTURE_DIRS, js_package_name_for_dir, js_resolve_relative, js_root_package,
};
use crate::languages::typescript::ts_type_token_score;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, TSX};
use crate::traits::{ImportExtractionMode, Language, TypeAnnotationStyle};
use crate::visibility::js_like_is_restricted;

pub struct Tsx;

fn load() -> TsLanguage {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "tsx",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Tsx {
    fn id(&self) -> LanguageId {
        TSX
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &TSX_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        js_root_package(specifier)
    }
    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        js_package_name_for_dir(dir)
    }
    fn imports_query(&self) -> Option<&'static str> {
        Some(include_str!("queries/imports.scm"))
    }
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn test_file_dirs(&self) -> &'static [&'static str] {
        JS_TEST_FILE_DIRS
    }
    fn test_file_suffixes(&self) -> &'static [&'static str] {
        JS_TEST_FILE_SUFFIXES
    }
    fn test_fixture_dirs(&self) -> &'static [&'static str] {
        JS_TEST_FIXTURE_DIRS
    }
    fn is_restricted(&self, source_text: &str, _name: &str) -> bool {
        js_like_is_restricted(source_text)
    }
    fn resolve_relative_specifier(
        &self,
        specifier: &str,
        from_file: &str,
        source_files: &HashSet<String>,
    ) -> Option<String> {
        js_resolve_relative(specifier, from_file, source_files)
    }
    fn conventional_dirs(&self) -> &'static [&'static str] {
        JS_CONVENTIONAL_DIRS
    }
    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &["index.tsx", "index.ts"]
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.7)
    }
    fn type_annotation_style(&self) -> TypeAnnotationStyle {
        TypeAnnotationStyle::ColonAfterName
    }
    fn uses_colon_return_suffix(&self) -> bool {
        true
    }
    fn type_token_score(&self, token: &str) -> f64 {
        ts_type_token_score(token)
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        crate::languages::typescript::TypeScript.idiomatic_symbol_names()
    }
}
