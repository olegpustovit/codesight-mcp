//! Sass preprocessor (no grammar).

use crate::languages::builtins::SASS_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, SASS};
use crate::traits::Language;

pub struct SassSupport;

impl Language for SassSupport {
    fn id(&self) -> LanguageId {
        SASS
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &SASS_MANIFEST
    }
}
