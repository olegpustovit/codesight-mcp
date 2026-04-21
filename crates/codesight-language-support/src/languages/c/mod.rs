//! C language (no tree-sitter grammar; shares `.h` header with C++).

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::C_MANIFEST;
use crate::manifest::{self, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct C;

impl Language for C {
    fn id(&self) -> LanguageId {
        manifest::C
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &C_MANIFEST
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
}
