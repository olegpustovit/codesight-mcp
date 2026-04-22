//! Elixir language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::ELIXIR_MANIFEST;
use crate::manifest::{ELIXIR, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct Elixir;

fn load() -> TsLanguage {
    tree_sitter_elixir::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "elixir",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

fn elixir_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    trimmed.starts_with("defp ") || trimmed.starts_with("defmacrop ")
}

impl Language for Elixir {
    fn id(&self) -> LanguageId {
        ELIXIR
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &ELIXIR_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_restricted(&self, src: &str, name: &str) -> bool {
        elixir_is_restricted(src, name)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::FunctionalExpression
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.3)
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        // OTP-pattern callbacks and protocol implementations repeat
        // across every GenServer / GenStateMachine / Supervisor in a
        // project by design.
        &[
            "start_link",
            "start",
            "init",
            "handle_call",
            "handle_cast",
            "handle_info",
            "handle_continue",
            "handle_event",
            "terminate",
            "code_change",
            "child_spec",
            "new",
            "call",
            "cast",
            "reply",
            "stop",
        ]
    }
}
