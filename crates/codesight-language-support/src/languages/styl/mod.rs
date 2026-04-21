//! Stylus preprocessor (no grammar).

use crate::languages::builtins::STYL_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, STYL};
use crate::traits::Language;

pub struct StylSupport;

impl Language for StylSupport {
    fn id(&self) -> LanguageId {
        STYL
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &STYL_MANIFEST
    }
}
