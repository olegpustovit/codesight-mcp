//! Functional-expression family — Elixir / Erlang.

use crate::traits::complexity::{ComplexityModel, LanguageFamily, NestingMode};
use crate::traits::liveness::LivenessPolicy;
use crate::traits::visibility::VisibilityModel;

pub struct Elixir;
pub struct Erlang;

fn elixir_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    trimmed.starts_with("defp ") || trimmed.starts_with("defmacrop ")
}

impl LivenessPolicy for Elixir {}
impl VisibilityModel for Elixir {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        elixir_is_restricted(source_text, name)
    }
}
impl ComplexityModel for Elixir {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::FunctionalExpression
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.3)
    }
}

impl LivenessPolicy for Erlang {}
impl VisibilityModel for Erlang {
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        elixir_is_restricted(source_text, name)
    }
}
impl ComplexityModel for Erlang {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::FunctionalExpression
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.3)
    }
}
