//! Gleam language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::GLEAM_MANIFEST;
use crate::manifest::{GLEAM, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct GleamSupport;

fn load() -> TsLanguage {
    tree_sitter_gleam::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "gleam",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for GleamSupport {
    fn id(&self) -> LanguageId {
        GLEAM
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &GLEAM_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
