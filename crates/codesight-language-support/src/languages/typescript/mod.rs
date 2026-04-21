//! TypeScript language.

pub mod nextjs;

use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::TYPESCRIPT_MANIFEST;
use crate::languages::javascript::{
    JS_ENTRYPOINTS, js_is_restricted, js_is_test_file, js_package_name_for_dir, js_root_package,
};
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, TYPESCRIPT};
use crate::signature_density::measure_colon_after_name;
use crate::traits::{ImportExtractionMode, Language};

pub struct TypeScript;

fn load() -> TsLanguage {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "typescript",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: Some(include_str!("queries/injections.scm")),
};

impl Language for TypeScript {
    fn id(&self) -> LanguageId {
        TYPESCRIPT
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &TYPESCRIPT_MANIFEST
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
        Some(0.7)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, "typescript", false, |_| false)
    }
}
