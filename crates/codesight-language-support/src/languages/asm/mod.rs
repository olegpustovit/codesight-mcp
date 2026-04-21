//! Assembly language (multi-dialect, no grammar).

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
}
