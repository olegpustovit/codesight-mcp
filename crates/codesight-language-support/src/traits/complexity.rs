//! Complexity / type-density axis (LANGUAGES.md §4.6).
//!
//! Owns: language family, nesting mode, type-density heuristics, signature
//! shape, weak-vs-exact typing. Replaces the scattered `language_profile`,
//! `measured_type_density`, `measure_go_signature`, and
//! `measure_php_signature` tables in `codesight-analysis::agent_readiness`.

/// Broad family a language belongs to, used by nesting-depth, function-length,
/// and type-density heuristics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LanguageFamily {
    BracedStatic,
    ScriptedDynamic,
    FunctionalExpression,
    DeclarativeMarkup,
    Unknown,
}

/// How this language expresses nesting (so the scanner knows whether to count
/// `{}` or significant indentation).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NestingMode {
    Brace,
    Indent,
}

pub trait ComplexityModel: Send + Sync {
    /// Rough family tag — see [`LanguageFamily`].
    fn family(&self) -> LanguageFamily {
        LanguageFamily::Unknown
    }

    /// How nesting depth is counted for this language.
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }

    /// Soft function-length warning threshold (lines). `None` when the
    /// language doesn't have a family-wide expectation (declarative/unknown).
    fn function_length_warn(&self) -> Option<u32> {
        match self.family() {
            LanguageFamily::BracedStatic => Some(75),
            LanguageFamily::ScriptedDynamic => Some(40),
            LanguageFamily::FunctionalExpression => Some(30),
            LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
        }
    }

    /// Baseline type-annotation score when the language does not support
    /// measured types or when the measurement returns `None`.
    fn fallback_type_baseline(&self) -> Option<f64> {
        match self.family() {
            LanguageFamily::BracedStatic => Some(1.0),
            LanguageFamily::ScriptedDynamic => Some(0.3),
            LanguageFamily::FunctionalExpression => Some(0.5),
            LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
        }
    }

    /// Whether `measured_type_density` returns a meaningful score for this
    /// language. `false` means agent-readiness falls back to
    /// `fallback_type_baseline`.
    fn supports_measured_types(&self) -> bool {
        false
    }

    /// Compute a 0..=1 type-annotation density for a single function body.
    /// Returns `None` when the signature can't be parsed (callers then use
    /// `fallback_type_baseline`).
    fn measured_type_density(&self, _source: &str) -> Option<f64> {
        None
    }
}

/// Default: unknown family, indent-based nesting, no measured types.
pub struct DefaultComplexity;

impl ComplexityModel for DefaultComplexity {}
