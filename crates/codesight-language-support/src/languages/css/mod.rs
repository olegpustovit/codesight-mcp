//! CSS language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::CSS_MANIFEST;
use crate::manifest::{CSS, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct CssSupport;

fn load() -> TsLanguage {
    tree_sitter_css::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "css",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for CssSupport {
    fn id(&self) -> LanguageId {
        CSS
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &CSS_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
