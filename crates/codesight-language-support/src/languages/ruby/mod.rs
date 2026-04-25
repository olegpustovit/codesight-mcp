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
    fn test_file_suffixes(&self) -> &'static [&'static str] {
        &["_spec.rb"]
    }
    fn test_file_dirs(&self) -> &'static [&'static str] {
        &["/spec/"]
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
            "if", "elsif", "unless", "while", "until", "case", "when", "rescue", "and", "or",
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
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["#"]
    }
    fn regex_reachability_patterns(
        &self,
    ) -> &'static [(codesight_core::EdgeKind, &'static str)] {
        // Ruby: `require 'foo'`, `require_relative 'foo'`, `load 'foo'`,
        // `autoload :Foo, 'foo'`. Capture the symbol/path string.
        &[
            (
                codesight_core::EdgeKind::ImportStatement,
                r#"\brequire(?:_relative)?\s+['"]([^'"]+)['"]"#,
            ),
            (
                codesight_core::EdgeKind::ImportStatement,
                r#"\bload\s+['"]([^'"]+)['"]"#,
            ),
            (
                codesight_core::EdgeKind::ImportStatement,
                r#"\bautoload\s+:(\w+)"#,
            ),
        ]
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
    fn vendor_segments(&self) -> &'static [&'static str] {
        &[".bundle"]
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined?",
            "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
            "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then",
            "true", "undef", "unless", "until", "when", "while", "yield",
        ]
    }
}
