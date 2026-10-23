//! Bash language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::BASH_MANIFEST;
use crate::manifest::{BASH, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct Bash;

fn load() -> TsLanguage {
    tree_sitter_bash::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "bash",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Bash {
    fn id(&self) -> LanguageId {
        BASH
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &BASH_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
