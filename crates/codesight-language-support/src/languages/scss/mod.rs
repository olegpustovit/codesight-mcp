//! SCSS language.

pub mod extractor;

use anyhow::Result;
use codesight_core::StoredSymbol;
use tree_sitter::Language as TsLanguage;

use crate::languages::builtins::SCSS_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, SCSS};
use crate::traits::Language;

pub struct ScssSupport;

fn load() -> TsLanguage {
    tree_sitter_scss::language()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "scss",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for ScssSupport {
    fn id(&self) -> LanguageId {
        SCSS
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &SCSS_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::ScssExtractor.extract(file_path, source)
    }
}
