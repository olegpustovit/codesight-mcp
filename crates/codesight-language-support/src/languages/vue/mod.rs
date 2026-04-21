//! Vue wrapper language + Nuxt framework providers.

pub mod imports;
pub mod nuxt;

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
}
