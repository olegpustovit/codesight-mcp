//! Swift language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::SWIFT_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, SWIFT};
use crate::traits::{Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

pub struct Swift;

fn load() -> TsLanguage {
    tree_sitter_swift::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "swift",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Swift {
    fn id(&self) -> LanguageId {
        SWIFT
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &SWIFT_MANIFEST
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
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "init",
            "deinit",
            "description",
            "hash",
            "copy",
            "main",
            "viewDidLoad",
            "viewWillAppear",
            "viewDidAppear",
            "viewWillDisappear",
            "viewDidDisappear",
            "body",
            "makeUIView",
            "updateUIView",
            "makeUIViewController",
            "updateUIViewController",
        ]
    }
}
