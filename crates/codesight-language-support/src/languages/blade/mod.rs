//! PHP-Blade templates (container-boundary extraction, no grammar).

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::BLADE_MANIFEST;
use crate::manifest::{BLADE, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct BladeSupport;

impl Language for BladeSupport {
    fn id(&self) -> LanguageId {
        BLADE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &BLADE_MANIFEST
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
}
