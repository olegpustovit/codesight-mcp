//! Razor templates (container-boundary extraction, no grammar).

pub mod extractor;

use anyhow::Result;
use dowser_core::StoredSymbol;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::RAZOR_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, RAZOR};
use crate::traits::Language;

pub struct RazorSupport;

impl Language for RazorSupport {
    fn id(&self) -> LanguageId {
        RAZOR
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &RAZOR_MANIFEST
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::RazorExtractor.extract(file_path, source)
    }
}
