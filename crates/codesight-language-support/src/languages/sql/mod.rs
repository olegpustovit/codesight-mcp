//! SQL language + dbt framework providers.

pub mod dbt;
pub mod extractor;
pub mod imports;

use anyhow::Result;
use codesight_core::StoredSymbol;
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
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::SqlExtractor.extract(file_path, source)
    }

    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["--"]
    }
    fn cyclomatic_complexity(&self, source: &str) -> usize {
        // SQL is tagged `DeclarativeMarkup` (family), but it has real
        // branching: `CASE WHEN`, `IF`, `IIF`, `ELSEIF`. Override the
        // family short-circuit with a case-insensitive scan.
        let lower = source.to_ascii_lowercase();
        crate::complexity::count_cyclomatic(&lower, &["if", "when", "elseif", "iif"], &[], &["--"])
    }

    fn normalise_body_for_dedup(&self, source: &str) -> Vec<u8> {
        // SQL is case-insensitive for keywords and identifiers — two queries
        // that differ only in keyword case must hash to the same value.
        // Apply the generic normalization (strip `--` line comments, trim,
        // collapse blanks) over a lower-cased copy of the source.
        let mut out = String::with_capacity(source.len());
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&trimmed.to_ascii_lowercase());
        }
        out.into_bytes()
    }
}
