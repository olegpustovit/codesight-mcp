//! SCM query-based extractor engine.
//!
//! Each language's `queries/tags.scm` (+ optional `injections.scm` +
//! `pack.toml`) feeds [`ScmExtractor`], which compiles tree-sitter queries
//! lazily on first use. The default body of [`crate::traits::Language::extract_symbols`]
//! routes curated languages through this engine via [`extract_with_spec`].

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

pub use crate::query_pack_catalog::{ScmMetadata, parse_scm_header, query_files_slice};

use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use dashmap::DashMap;
use tracing::warn;

use dowser_core::StoredSymbol;

use crate::manifest::GrammarSpec;
use crate::query_pack_catalog::{QueryPackCatalog, QueryPackDefinition};
use load::CompiledQueryPack;

// ---------------------------------------------------------------------------
// ScmExtractor (facade)
// ---------------------------------------------------------------------------

/// SCM-query-driven extractor for one language pack.
///
/// `ScmExtractor` is cheap to construct: it stores the raw `.scm` sources and
/// metadata but does **not** run `tree_sitter::Query::new` until the first
/// call to [`Self::compiled`] (via `extract`).
pub struct ScmExtractor {
    pub(in crate::scm_engine) language_name: &'static str,
    pub(in crate::scm_engine) extensions: &'static [&'static str],
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
        if crate::builtin_grammar(&meta.grammar).is_none() {
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
            grammar_name: meta.grammar.clone(),
            query_source,
            injections_source,
            compiled: OnceLock::new(),
        })
    }

    pub fn language_name(&self) -> &'static str {
        self.language_name
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    pub fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extract::extract_symbols(self, file_path, source)
    }

    pub(in crate::scm_engine) fn compiled(&self) -> &CompiledQueryPack {
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
        let _ = self.compiled.set(pack);
        self.compiled.get().expect("just set")
    }
}

/// Run a lazily-compiled SCM pack against `source`. Used by the default
/// `Language::extract_symbols` impl for every curated-SCM language.
///
/// The hot path is a single DashMap shard lock + string lookup — no write
/// contention between parallel ingest workers once each language is cached.
pub fn extract_with_spec(
    language_name: &str,
    spec: &'static GrammarSpec,
    file_path: &str,
    source: &str,
) -> Result<Vec<StoredSymbol>> {
    static CACHE: OnceLock<DashMap<String, Arc<ScmExtractor>>> = OnceLock::new();
    let cache = CACHE.get_or_init(DashMap::new);

    if let Some(existing) = cache.get(language_name) {
        return existing.extract(file_path, source);
    }

    let meta = ScmMetadata {
        name: language_name.to_string(),
        grammar: spec.grammar_name.to_string(),
        extensions: Vec::new(),
        override_extractor: None,
    };
    let extractor = Arc::new(ScmExtractor::build(
        &meta,
        spec.tags_scm.to_string(),
        spec.injections_scm.map(|s| s.to_string()),
    )?);

    // `entry(...).or_insert_with(...)` is atomic per shard — racing callers
    // for the same language share the winner's Arc.
    let cached = cache
        .entry(language_name.to_string())
        .or_insert_with(|| extractor)
        .clone();
    cached.extract(file_path, source)
}

pub fn build_scm_extractors() -> Result<Vec<Box<ScmExtractor>>> {
    build_scm_extractors_from_catalog(&QueryPackCatalog::builtin())
}

pub fn build_scm_extractors_from_catalog(
    catalog: &QueryPackCatalog,
) -> Result<Vec<Box<ScmExtractor>>> {
    let mut extractors: Vec<Box<ScmExtractor>> = Vec::new();

    for pack in catalog.iter() {
        if pack.metadata.override_extractor.is_some() {
            continue;
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
