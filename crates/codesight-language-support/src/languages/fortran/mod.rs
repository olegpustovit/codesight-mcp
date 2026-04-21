//! Fortran language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::FORTRAN_MANIFEST;
use crate::manifest::{FORTRAN, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct FortranSupport;

fn load() -> TsLanguage {
    tree_sitter_fortran::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "fortran",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for FortranSupport {
    fn id(&self) -> LanguageId {
        FORTRAN
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &FORTRAN_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
