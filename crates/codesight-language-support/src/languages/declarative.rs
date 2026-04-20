//! Declarative-markup family (JSON, XML, TOML, SQL, YAML, Blade, Razor).
//!
//! These languages do not participate in function-level complexity or
//! visibility analysis. They register as declarative-markup so agent-readiness
//! knows to skip them.

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};

pub struct DeclarativeMarkup;

impl ComplexityModel for DeclarativeMarkup {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
}
