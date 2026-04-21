//! Razor templates (container-boundary extraction, no grammar).

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
}
