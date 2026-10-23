//! Vue wrapper language + Nuxt framework providers.

pub mod extractor;
pub mod imports;
pub mod nuxt;

use anyhow::Result;
use dowser_core::StoredSymbol;

use crate::imports_types::ImportEdge;
use crate::languages::builtins::VUE_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, VUE};
use crate::traits::{ImportExtractionMode, Language};

pub struct VueSupport;

// Vue has no own tree-sitter grammar; override extract_imports directly.

// Vue has no own tree-sitter grammar — relies on container-boundary extraction
// residual + fallback-only imports.

impl Language for VueSupport {
    fn id(&self) -> LanguageId {
        VUE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &VUE_MANIFEST
    }
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::FallbackOnly)
    }
    fn extract_imports(&self, source: &str) -> Vec<ImportEdge> {
        imports::extract(source)
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::VueExtractor.extract(file_path, source)
    }
    fn vendor_segments(&self) -> &'static [&'static str] {
        crate::languages::javascript::JS_VENDOR_SEGMENTS
    }
}
