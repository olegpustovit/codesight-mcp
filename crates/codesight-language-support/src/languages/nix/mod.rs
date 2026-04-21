//! Nix language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::NIX_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, NIX};
use crate::traits::Language;

pub struct NixSupport;

fn load() -> TsLanguage {
    tree_sitter_nix::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "nix",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for NixSupport {
    fn id(&self) -> LanguageId {
        NIX
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &NIX_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
