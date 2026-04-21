//! EJS templates.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::EJS_MANIFEST;
use crate::manifest::{EJS, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct EjsSupport;

fn load() -> TsLanguage {
    tree_sitter_embedded_template::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "embedded_template",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: Some(include_str!("queries/injections.scm")),
};

impl Language for EjsSupport {
    fn id(&self) -> LanguageId {
        EJS
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &EJS_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
