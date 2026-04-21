//! Luau language.

pub mod extractor;

use anyhow::Result;
use codesight_core::StoredSymbol;
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
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::LuauExtractor.extract(file_path, source)
    }
}
