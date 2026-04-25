//! Kotlin language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::KOTLIN_MANIFEST;
use crate::manifest::{GrammarSpec, KOTLIN, LanguageId, LanguageManifest};
use crate::traits::{Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

pub struct Kotlin;

fn load() -> TsLanguage {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "kotlin",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Kotlin {
    fn id(&self) -> LanguageId {
        KOTLIN
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &KOTLIN_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
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
        TypeAnnotationStyle::ColonAfterName
    }
    fn uses_colon_return_suffix(&self) -> bool {
        true
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["any"]
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "when", "catch"]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "equals",
            "hashCode",
            "toString",
            "compareTo",
            "invoke",
            "iterator",
            "hasNext",
            "next",
            "component1",
            "component2",
            "component3",
            "copy",
            "plus",
            "minus",
            "times",
            "div",
            "get",
            "set",
            "contains",
            "close",
            "main",
            "onCreate",
            "onDestroy",
            "onStart",
            "onStop",
            "onResume",
            "onPause",
        ]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        id.leaf_name().starts_with("test")
    }
    fn vendor_segments(&self) -> &'static [&'static str] {
        &[".gradle", ".idea"]
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in",
            "interface", "is", "null", "object", "package", "return", "super", "this", "throw",
            "true", "try", "typealias", "typeof", "val", "var", "when", "while", "by", "catch",
            "constructor", "delegate", "dynamic", "field", "file", "finally", "get", "import",
            "init", "param", "property", "receiver", "set", "setparam", "where", "actual",
            "abstract", "annotation", "companion", "const", "crossinline", "data", "enum",
            "expect", "external", "final", "infix", "inline", "inner", "internal", "lateinit",
            "noinline", "open", "operator", "out", "override", "private", "protected", "public",
            "reified", "sealed", "suspend", "tailrec", "vararg",
        ]
    }
}
