//! JSX language — re-registers JavaScript grammar/behavior for `.jsx` files.

use std::path::Path;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JSX_MANIFEST;
use crate::languages::javascript::{
    GRAMMAR as JS_GRAMMAR, JS_ENTRYPOINTS, js_is_restricted, js_is_test_file,
    js_package_name_for_dir, js_root_package,
};
use crate::manifest::{GrammarSpec, JSX, LanguageId, LanguageManifest};
use crate::signature_density::measure_colon_after_name;
use crate::traits::{ImportExtractionMode, Language};

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
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
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
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, self, false, |_| false)
    }
}
