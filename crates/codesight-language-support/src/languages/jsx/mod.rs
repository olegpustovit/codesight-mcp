//! JSX language — re-registers JavaScript grammar/behavior for `.jsx` files.

use rustc_hash::FxHashSet;
use std::path::Path;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JSX_MANIFEST;
use crate::languages::javascript::{
    GRAMMAR as JS_GRAMMAR, JS_CONVENTIONAL_DIRS, JS_ENTRYPOINTS, JS_RESERVED_IDENTIFIERS,
    JS_TEST_FILE_DIRS, JS_TEST_FILE_SUFFIXES, JS_TEST_FIXTURE_DIRS, JS_VENDOR_SEGMENTS,
    js_package_name_for_dir, js_resolve_relative, js_root_package,
};
use crate::manifest::{GrammarSpec, JSX, LanguageId, LanguageManifest};
use crate::traits::{ImportExtractionMode, Language, TypeAnnotationStyle};
use crate::visibility::js_like_is_restricted;

pub struct Jsx;

impl Language for Jsx {
    fn id(&self) -> LanguageId {
        JSX
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &JSX_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&JS_GRAMMAR)
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
        // Reuse JavaScript's SCM — JSX parses with the same grammar.
        Some(include_str!("../javascript/queries/imports.scm"))
    }
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn vendor_segments(&self) -> &'static [&'static str] {
        JS_VENDOR_SEGMENTS
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        JS_RESERVED_IDENTIFIERS
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
        source_files: &FxHashSet<String>,
    ) -> Option<String> {
        js_resolve_relative(specifier, from_file, source_files)
    }
    fn conventional_dirs(&self) -> &'static [&'static str] {
        JS_CONVENTIONAL_DIRS
    }
    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &["index.jsx", "index.js"]
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
    fn type_annotation_style(&self) -> TypeAnnotationStyle {
        TypeAnnotationStyle::ColonAfterName
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        crate::languages::javascript::JavaScript.idiomatic_symbol_names()
    }
}
