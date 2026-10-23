//! AL language (regex-only, no grammar).

pub mod extractor;

use anyhow::Result;
use symslice_core::StoredSymbol;

use crate::languages::builtins::AL_MANIFEST;
use crate::manifest::{AL, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct AlSupport;

impl Language for AlSupport {
    fn id(&self) -> LanguageId {
        AL
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &AL_MANIFEST
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::AlExtractor.extract(file_path, source)
    }
}
