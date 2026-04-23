//! Erlang language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::ERLANG_MANIFEST;
use crate::manifest::{ERLANG, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;
use crate::visibility::beam_is_restricted;

pub struct Erlang;

fn load() -> TsLanguage {
    tree_sitter_erlang::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "erlang",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Erlang {
    fn id(&self) -> LanguageId {
        ERLANG
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &ERLANG_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_restricted(&self, src: &str, _name: &str) -> bool {
        beam_is_restricted(src)
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
}
