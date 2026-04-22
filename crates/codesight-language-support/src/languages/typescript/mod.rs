//! TypeScript language.

pub mod nextjs;
mod simple_framework;

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::TYPESCRIPT_MANIFEST;
use crate::languages::javascript::{
    JS_ENTRYPOINTS, js_is_restricted, js_is_test_file, js_package_name_for_dir, js_root_package,
};
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, TYPESCRIPT};
use crate::signature_density::{measure_colon_after_name, normalize_type_token};
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
    fn pruned_dirs(&self) -> &'static [&'static str] {
        crate::languages::javascript::JS_PRUNED_DIRS
    }
    fn simple_framework_profile(
        &self,
        root: &Path,
    ) -> Option<crate::frameworks::profiles::FrameworkProfile> {
        simple_framework::detect(root)
    }
    fn imports_query(&self) -> Option<&'static str> {
        Some(include_str!("queries/imports.scm"))
    }
    fn path_aliases(&self, root: &Path) -> HashMap<String, Vec<String>> {
        crate::imports_orchestration::load_tsconfig_aliases(&root.to_string_lossy())
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "catch", "case"]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "constructor",
            "toString",
            "valueOf",
            "toJSON",
            "then",
            "catch",
            "finally",
            "render",
            "componentDidMount",
            "componentWillUnmount",
            "componentDidUpdate",
            "shouldComponentUpdate",
            "getDerivedStateFromProps",
            "handleClick",
            "handleChange",
            "handleSubmit",
            "default",
            "index",
            "main",
            "init",
            "setup",
            "teardown",
            "beforeEach",
            "afterEach",
            "beforeAll",
            "afterAll",
            "describe",
            "it",
            "test",
            "ngOnInit",
            "ngOnDestroy",
            "ngOnChanges",
            "ngAfterViewInit",
        ]
    }
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &[
            "import ",
            "export ",
            "function ",
            "class ",
            "interface ",
            "type ",
            "enum ",
        ]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        let name = id.leaf_name().to_ascii_lowercase();
        name.starts_with("test") || name.ends_with("spec")
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
        measure_colon_after_name(source, self, false, |_| false)
    }
    fn uses_colon_return_suffix(&self) -> bool {
        true
    }
    fn type_token_score(&self, token: &str) -> f64 {
        ts_type_token_score(token)
    }
}

/// TypeScript / TSX weak-type scoring — `any` / `object` collapse to zero,
/// `unknown` stays at 0.3 (narrower than `any` but still an escape hatch).
pub(crate) fn ts_type_token_score(token: &str) -> f64 {
    let normalized = normalize_type_token(token);
    if normalized.is_empty() {
        return 0.0;
    }
    if normalized == "unknown" {
        return 0.3;
    }
    if matches!(normalized.as_str(), "any" | "object") || normalized.contains("<any>") {
        return 0.0;
    }
    1.0
}
