//! Kotlin language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::KOTLIN_MANIFEST;
use crate::manifest::{GrammarSpec, KOTLIN, LanguageId, LanguageManifest};
use crate::signature_density::{measure_colon_after_name, normalize_type_token};
use crate::traits::Language;

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

fn cfamily_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
    {
        return true;
    }
    !trimmed.starts_with("public ")
}

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
    fn is_restricted(&self, src: &str, name: &str) -> bool {
        cfamily_is_restricted(src, name)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, src: &str) -> Option<f64> {
        measure_colon_after_name(src, self, false, |_| false)
    }
    fn uses_colon_return_suffix(&self) -> bool {
        true
    }
    fn type_token_score(&self, token: &str) -> f64 {
        let normalized = normalize_type_token(token);
        if normalized.is_empty() {
            return 0.0;
        }
        if normalized == "any" {
            return 0.0;
        }
        1.0
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "when", "catch"]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        id.leaf_name().starts_with("test")
    }
}
