//! TOML configuration language.

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::TOML_MANIFEST;
use crate::manifest::{LanguageId, LanguageManifest, TOML};
use crate::traits::Language;

pub struct TomlSupport;

impl Language for TomlSupport {
    fn id(&self) -> LanguageId {
        TOML
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &TOML_MANIFEST
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
}
