//! Assembly language (multi-dialect, no grammar).

pub mod extractor;

use anyhow::Result;
use dowser_core::StoredSymbol;

use crate::languages::builtins::ASM_MANIFEST;
use crate::manifest::{ASM, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct AsmSupport;

impl Language for AsmSupport {
    fn id(&self) -> LanguageId {
        ASM
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &ASM_MANIFEST
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::AsmExtractor.extract(file_path, source)
    }
}
