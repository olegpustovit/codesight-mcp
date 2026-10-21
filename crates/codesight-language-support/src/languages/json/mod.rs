//! JSON language + host profiles.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JSON_MANIFEST;
use crate::manifest::{GrammarSpec, JSON, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct JsonSupport;

fn load() -> TsLanguage {
    tree_sitter_json::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "json",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for JsonSupport {
    fn id(&self) -> LanguageId {
        JSON
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &JSON_MANIFEST
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
    fn is_reference_target(&self) -> bool {
        false
    }
    fn postprocess_symbols(
        &self,
        symbols: &mut [codesight_core::StoredSymbol],
        file_path: &str,
        _source: &str,
    ) {
        for symbol in symbols {
            if let Ok(name) = serde_json::from_str::<String>(&symbol.name) {
                if let Some(prefix) = symbol.qualified_name.strip_suffix(&symbol.name) {
                    symbol.qualified_name = format!("{prefix}{name}");
                }
                symbol.id = format!(
                    "{file_path}::{}#{}",
                    symbol.qualified_name,
                    symbol.kind.as_str()
                );
                symbol.name = name;
            }
        }
    }
}

#[cfg(test)]
mod tests;
