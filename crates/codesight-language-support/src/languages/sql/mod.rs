//! SQL language + dbt framework providers.

pub mod dbt;
pub mod imports;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::SQL_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, SQL};
use crate::traits::{ImportExtractionMode, Language};

pub struct SqlSupport;

// SQL's dbt Jinja imports sit outside the SQL grammar, so override directly
// instead of declaring an imports.scm.

fn load() -> TsLanguage {
    tree_sitter_sequel::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "sql",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for SqlSupport {
    fn id(&self) -> LanguageId {
        SQL
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &SQL_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::FallbackOnly)
    }
    fn extract_imports(&self, source: &str) -> Vec<ImportEdge> {
        imports::extract(source)
    }
}
