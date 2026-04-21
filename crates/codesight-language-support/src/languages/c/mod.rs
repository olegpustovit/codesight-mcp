//! C language — shares the tree-sitter C++ grammar (which parses plain C
//! correctly). `.h` disambiguation between C and C++ is a content probe
//! attached to the C++ manifest, not the other way around.

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::C_MANIFEST;
use crate::languages::cpp;
use crate::manifest::{self, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct C;

impl Language for C {
    fn id(&self) -> LanguageId {
        manifest::C
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &C_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&cpp::GRAMMAR)
    }
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.c"]
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
}
