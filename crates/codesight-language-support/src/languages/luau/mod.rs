//! Luau language.

use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::LUAU_MANIFEST;
use crate::manifest::{GrammarSpec, LUAU, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct LuauSupport;

fn load() -> TsLanguage {
    tree_sitter_luau::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "luau",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for LuauSupport {
    fn id(&self) -> LanguageId {
        LUAU
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &LUAU_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
}
