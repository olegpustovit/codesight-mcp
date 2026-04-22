//! JavaScript language (JSX handled separately via `jsx` module).

use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JAVASCRIPT_MANIFEST;
use crate::manifest::{GrammarSpec, JAVASCRIPT, LanguageId, LanguageManifest};
use crate::signature_density::measure_colon_after_name;
use crate::traits::{ImportExtractionMode, Language};

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

pub(super) fn js_is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/__tests__/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/spec/")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".test.js")
        || lower.ends_with(".test.jsx")
        || lower.ends_with("_test.ts")
        || lower.ends_with("_test.js")
        || lower.ends_with("_spec.ts")
        || lower.ends_with("_spec.js")
}

pub(super) fn js_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ") || trimmed.starts_with("protected ") {
        return true;
    }
    !trimmed.starts_with("export ")
}

pub(super) const JS_ENTRYPOINTS: &[&str] = &[
    "main.js", "index.js", "app.js", "server.js", "main.ts", "index.ts", "app.ts", "server.ts",
];

pub(crate) const JS_PRUNED_DIRS: &[&str] = &["node_modules", "dist", "build", "out"];

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
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        JS_ENTRYPOINTS
    }
    fn is_test_file(&self, path: &str) -> bool {
        js_is_test_file(path)
    }
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        js_is_restricted(source_text, name)
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
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, self, false, |_| false)
    }
}
