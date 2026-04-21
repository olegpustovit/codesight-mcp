//! Less CSS preprocessor (no grammar).

use crate::languages::builtins::LESS_MANIFEST;
use crate::manifest::{LESS, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct LessSupport;

impl Language for LessSupport {
    fn id(&self) -> LanguageId {
        LESS
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &LESS_MANIFEST
    }
}
