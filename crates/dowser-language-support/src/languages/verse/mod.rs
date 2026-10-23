//! Verse language (regex-only, no grammar).

pub mod extractor;

use anyhow::Result;
use dowser_core::StoredSymbol;

use crate::languages::builtins::VERSE_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, VERSE};
use crate::traits::Language;

pub struct VerseSupport;

impl Language for VerseSupport {
    fn id(&self) -> LanguageId {
        VERSE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &VERSE_MANIFEST
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::VerseExtractor.extract(file_path, source)
    }
}
