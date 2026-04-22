//! Ruby language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::RUBY_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, RUBY};
use crate::traits::Language;

pub struct Ruby;

fn load() -> TsLanguage {
    tree_sitter_ruby::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "ruby",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Ruby {
    fn id(&self) -> LanguageId {
        RUBY
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &RUBY_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_test_file(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with("_spec.rb") || lower.contains("/spec/")
    }
    fn is_restricted(&self, _src: &str, name: &str) -> bool {
        name.starts_with('_')
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &[
            "if", "elsif", "unless", "while", "until", "case", "when",
            "rescue", "and", "or",
        ]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "initialize",
            "to_s",
            "to_str",
            "to_a",
            "to_h",
            "to_i",
            "to_f",
            "to_sym",
            "inspect",
            "hash",
            "eql?",
            "==",
            "<=>",
            "each",
            "each_with_index",
            "map",
            "select",
            "reject",
            "call",
            "new",
            "setup",
            "teardown",
            "before",
            "after",
            "describe",
            "context",
            "it",
            "let",
        ]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["#"]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        // RSpec (`describe`, `context`, `it`) + Minitest (`test_…`).
        let name = id.leaf_name();
        name.starts_with("test_")
            || matches!(
                id.parent_path().rsplit('.').next().unwrap_or(""),
                "describe" | "context" | "it"
            )
    }
}
