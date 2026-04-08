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
mod load;
mod metadata;
mod pattern;
mod postprocess;
mod scope;

#[cfg(test)]
mod tests;

pub use super::catalogs::QUERY_FILES;
pub use metadata::{ScmMetadata, parse_scm_header};

use anyhow::Result;

use super::catalogs::{QueryPackCatalog, QueryPackDefinition};
use crate::extractors::LanguageExtractor;
use crate::model::{ExtractionTier, StoredSymbol};
use load::CompiledQueryPack;

// ---------------------------------------------------------------------------
// ScmExtractor (facade)
// ---------------------------------------------------------------------------

pub struct ScmExtractor {
    compiled: CompiledQueryPack,
}

impl ScmExtractor {
    pub fn new(meta: &ScmMetadata, query_source: &str) -> Result<Self> {
        Ok(Self {
            compiled: load::compile_query_pack(meta, query_source)?,
        })
    }

    pub fn from_pack(pack: &QueryPackDefinition) -> Result<Self> {
        Ok(Self {
            compiled: load::compile_pack(pack)?,
        })
    }
}

impl LanguageExtractor for ScmExtractor {
    fn language_name(&self) -> &'static str {
        self.compiled.language_name
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        self.compiled.extensions
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
                eprintln!(
                    "warning: failed to compile query pack {}: {e}",
                    pack.pack_id
                );
            }
        }
    }

    Ok(extractors)
}
