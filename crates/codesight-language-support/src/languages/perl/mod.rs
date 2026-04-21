//! Perl language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::PERL_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, PERL};
use crate::traits::Language;

pub struct Perl;

fn load() -> TsLanguage {
    tree_sitter_perl::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "perl",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Perl {
    fn id(&self) -> LanguageId {
        PERL
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &PERL_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
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
}
