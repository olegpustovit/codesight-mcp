//! SCM query-based extractor engine.
//!
//! Built-in packs live under `runtime/queries/<pack_id>/tags.scm`, with optional
//! `injections.scm` and `pack.toml`. The pack catalog is generated at build time by `build.rs`;
//! [`super::catalogs::QueryPackCatalog`] loads those sources and hands them to
//! [`ScmExtractor`]. Submodules hold header parsing, pattern metadata, capture types, doc
//! cleanup, and the extract phases; [`ScmExtractor`] is the public façade over tree-sitter
//! queries.
//!
//! Language knowledge for symbol capture lives in `.scm` files. A small post-pass for C/C++
//! headers lives in [`super::cpp_header_language_fallback`].

mod capture;
mod docs;
mod extract;
mod injections;
mod load;
mod pattern;
mod postprocess;
mod scope;

#[cfg(test)]
mod tests;

pub use super::catalogs::{QUERY_FILES, ScmMetadata, parse_scm_header};

use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use tracing::warn;

use super::catalogs::{QueryPackCatalog, QueryPackDefinition};
use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::ExtractionTier;
use crate::extractors::LanguageExtractor;
use load::CompiledQueryPack;
use postprocess::{PostprocessStep, postprocess_steps_for};

// ---------------------------------------------------------------------------
// ScmExtractor (facade)
// ---------------------------------------------------------------------------

/// SCM-query-driven extractor for one language pack.
///
/// `ScmExtractor` is cheap to construct: it stores the raw `.scm` sources and
/// metadata but does **not** run `tree_sitter::Query::new` until the first call
/// to [`Self::compiled`] (via `extract`). For a 30-pack registry this turns
/// ~1.25 s of eager query compilation into ~40 ms × (languages actually seen),
/// which is the dominant cold cost on the test suite (see worklog #17).
pub struct ScmExtractor {
    pub(in crate::extractors::engine::scm) language_name: &'static str,
    pub(in crate::extractors::engine::scm) extensions: &'static [&'static str],
    pub(in crate::extractors::engine::scm) postprocess_steps: &'static [PostprocessStep],
    grammar_name: String,
    query_source: String,
    injections_source: Option<String>,
    compiled: OnceLock<CompiledQueryPack>,
}

impl ScmExtractor {
    pub fn new(meta: &ScmMetadata, query_source: &str) -> Result<Self> {
        Self::build(meta, query_source.to_string(), None)
    }

    pub fn from_pack(pack: &QueryPackDefinition) -> Result<Self> {
        Self::build(
            &pack.metadata,
            pack.query_source.clone(),
            pack.injections_source.clone(),
        )
    }

    fn build(
        meta: &ScmMetadata,
        query_source: String,
        injections_source: Option<String>,
    ) -> Result<Self> {
        // Fail fast on unknown grammar so bad packs are still caught at
        // registry-construction time — tree-sitter grammar lookup is a
        // process-static map copy, cheap compared with `Query::new`.
        if crate::extractors::engine::builtin_grammar(&meta.grammar).is_none() {
            return Err(anyhow!("unknown grammar: {}", meta.grammar));
        }
        let language_name: &'static str = Box::leak(meta.name.clone().into_boxed_str());
        let extensions: &'static [&'static str] = Box::leak(
            meta.extensions
                .iter()
                .map(|ext| &*Box::leak(ext.clone().into_boxed_str()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        Ok(Self {
            language_name,
            extensions,
            postprocess_steps: postprocess_steps_for(meta),
            grammar_name: meta.grammar.clone(),
            query_source,
            injections_source,
            compiled: OnceLock::new(),
        })
    }

    /// Returns the compiled tree-sitter queries, compiling them on first access.
    /// Panics only on a broken pack — which is caught eagerly by
    /// `scm_all_queries_compile` tests, so production never hits this.
    pub(in crate::extractors::engine::scm) fn compiled(&self) -> &CompiledQueryPack {
        if let Some(c) = self.compiled.get() {
            return c;
        }
        let pack = load::compile_queries(
            &self.grammar_name,
            &self.query_source,
            self.injections_source.as_deref(),
            self.language_name,
        )
        .unwrap_or_else(|err| panic!("lazy SCM compile failed for {}: {err}", self.language_name));
        // Race-safe: if another thread beat us, its pack is equivalent.
        let _ = self.compiled.set(pack);
        self.compiled.get().expect("just set")
    }
}

impl LanguageExtractor for ScmExtractor {
    fn language_name(&self) -> &'static str {
        self.language_name
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::CuratedQueryPack
    }

    fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extract::extract_symbols(self, file_path, source)
    }
}

pub fn build_scm_extractors() -> Result<Vec<Box<dyn LanguageExtractor>>> {
    build_scm_extractors_from_catalog(&QueryPackCatalog::builtin())
}

pub fn build_scm_extractors_from_catalog(
    catalog: &QueryPackCatalog,
) -> Result<Vec<Box<dyn LanguageExtractor>>> {
    let mut extractors: Vec<Box<dyn LanguageExtractor>> = Vec::new();

    for pack in catalog.iter() {
        if pack.metadata.override_extractor.is_some() {
            continue; // handled by override registry
        }
        if pack.metadata.grammar.is_empty() {
            continue;
        }
        match ScmExtractor::from_pack(pack) {
            Ok(ext) => extractors.push(Box::new(ext)),
            Err(e) => {
                warn!(
                    pack_id = %pack.pack_id,
                    error = %e,
                    "failed to build query pack"
                );
            }
        }
    }

    Ok(extractors)
}
