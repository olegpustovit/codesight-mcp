//! Language-family classification + nesting mode enums.
//!
//! Shared data, not behavior: both live here because multiple languages pick
//! a family and the family alone drives a handful of default thresholds via
//! [`crate::fallback`]. The old `braced_static.rs` / `declarative.rs` /
//! `functional.rs` / `scripted_dynamic.rs` per-family files are gone — each
//! language declares its own family inline.

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
