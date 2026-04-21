//! Verse language (regex-only, no grammar).

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
}
