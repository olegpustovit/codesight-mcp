//! AL language (regex-only, no grammar).

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
}
