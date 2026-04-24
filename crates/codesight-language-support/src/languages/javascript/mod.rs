//! JavaScript language (JSX handled separately via `jsx` module).

pub mod imports;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JAVASCRIPT_MANIFEST;
use crate::manifest::{GrammarSpec, JAVASCRIPT, LanguageId, LanguageManifest};
use crate::traits::{ImportExtractionMode, Language, TypeAnnotationStyle};
use crate::visibility::js_like_is_restricted;

pub(crate) fn js_resolve_relative(
    specifier: &str,
    from_file: &str,
    source_files: &HashSet<String>,
) -> Option<String> {
    imports::resolve_js_relative(specifier, from_file, source_files)
}

pub struct JavaScript;

fn load() -> TsLanguage {
    tree_sitter_javascript::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "javascript",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: Some(include_str!("queries/injections.scm")),
};

pub(super) fn js_root_package(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    if specifier.starts_with('@') {
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return format!("{}/{}", parts[0], parts[1]);
        }
        return specifier.to_string();
    }
    specifier.split('/').next().unwrap_or("").to_string()
}

pub(super) fn js_package_name_for_dir(dir: &Path) -> Option<String> {
    let package_json = dir.join("package.json");
    if !package_json.is_file() {
        return None;
    }
    let content = fs::read_to_string(&package_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

pub(crate) const JS_TEST_FILE_DIRS: &[&str] = &["/__tests__/", "/tests/", "/test/", "/spec/"];

pub(crate) const JS_TEST_FILE_SUFFIXES: &[&str] = &[
    ".test.ts",
    ".test.tsx",
    ".test.js",
    ".test.jsx",
    "_test.ts",
    "_test.js",
    "_spec.ts",
    "_spec.js",
];

pub(crate) const JS_TEST_FIXTURE_DIRS: &[&str] = &["/__snapshots__/"];

pub(super) const JS_ENTRYPOINTS: &[&str] = &[
    "main.js",
    "index.js",
    "app.js",
    "server.js",
    "main.ts",
    "index.ts",
    "app.ts",
    "server.ts",
];

pub(crate) const JS_PRUNED_DIRS: &[&str] = &["node_modules", "dist", "build", "out"];

pub(crate) const JS_CONVENTIONAL_DIRS: &[&str] = &[
    "tests/",
    "__tests__/",
    "spec/",
    "fixtures/",
    "scripts/",
    "examples/",
];

impl Language for JavaScript {
    fn id(&self) -> LanguageId {
        JAVASCRIPT
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &JAVASCRIPT_MANIFEST
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
    fn imports_query(&self) -> Option<&'static str> {
        Some(include_str!("queries/imports.scm"))
    }
    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        js_package_name_for_dir(dir)
    }
    fn pruned_dirs(&self) -> &'static [&'static str] {
        JS_PRUNED_DIRS
    }
    fn outline_noise_basenames(&self) -> &'static [&'static str] {
        &["package.json", "package-lock.json"]
    }
    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &["index.js", "index.mjs", "index.cjs"]
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
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "catch", "case"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &["import ", "export ", "function ", "class "]
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
            "getInitialState",
            "getDefaultProps",
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
        ]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        let name = id.leaf_name().to_ascii_lowercase();
        name.starts_with("test") || name.ends_with("spec")
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
}
