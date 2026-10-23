//! Thin adapter over `symslice-language-support::grammars`.
//!
//! Phase C moved grammar ownership into `symslice-language-support`. This
//! module keeps the `GrammarCatalog` / `GrammarDefinition` surface the
//! extractor tier was built against, delegating every lookup to the
//! language crate. The extractors crate no longer owns a private grammar
//! table.

use tree_sitter::Language;

use crate::builtin_registry;
use crate::grammars as ls_grammars;
use crate::manifest::GrammarSpec;

#[derive(Clone, Copy)]
pub struct GrammarDefinition {
    spec: &'static GrammarSpec,
    language_name: &'static str,
    extensions: &'static [&'static str],
}

impl GrammarDefinition {
    pub fn grammar_name(&self) -> &'static str {
        self.spec.grammar_name
    }

    pub fn language_name(&self) -> &'static str {
        self.language_name
    }

    pub fn extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    pub fn language(&self) -> Language {
        self.spec.language()
    }

    pub fn spec(&self) -> &'static GrammarSpec {
        self.spec
    }
}

pub struct GrammarCatalog {
    definitions: Vec<GrammarDefinition>,
}

impl GrammarCatalog {
    pub fn builtin() -> Self {
        let registry = builtin_registry();
        let definitions = ls_grammars::builtin_grammars()
            .iter()
            .map(|spec| {
                let (language_name, extensions) = registry
                    .all()
                    .find_map(|lang| {
                        let grammar = lang.manifest().grammar?;
                        (grammar.grammar_name == spec.grammar_name)
                            .then_some((lang.id().as_str(), lang.manifest().extensions))
                    })
                    .unwrap_or((spec.grammar_name, &[]));
                GrammarDefinition {
                    spec,
                    language_name,
                    extensions,
                }
            })
            .collect();
        Self { definitions }
    }

    pub fn find_by_grammar(&self, grammar_name: &str) -> Option<GrammarDefinition> {
        self.definitions
            .iter()
            .find(|entry| entry.spec.grammar_name == grammar_name)
            .copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = GrammarDefinition> + '_ {
        self.definitions.iter().copied()
    }
}
