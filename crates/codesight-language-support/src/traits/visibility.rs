//! Visibility axis (LANGUAGES.md §4.5).
//!
//! Owns the language-specific public/internal/private interpretation used by
//! agent-readiness, blast-radius, and rename-safety metrics. Phase D moves
//! `has_restricted_visibility` from `codesight-analysis::agent_readiness` into
//! per-language trait impls.

pub trait VisibilityModel: Send + Sync {
    /// True when the symbol at `source_text` with the given `name` is
    /// restricted (non-public) under this language's visibility model.
    /// `source_text` is the stored symbol source (typically starts with the
    /// declaration); `name` is the symbol identifier.
    fn is_restricted(&self, source_text: &str, name: &str) -> bool {
        // Fallback: underscore-prefix convention used by Python, Ruby, Perl,
        // Lua, and generic "unknown" languages.
        let _ = source_text;
        name.starts_with('_')
    }
}

/// Default: treat every symbol as publicly visible. Used by languages
/// without a formal visibility model (JSON, CSS, XML, …).
pub struct PermissiveVisibility;

impl VisibilityModel for PermissiveVisibility {
    fn is_restricted(&self, _source_text: &str, _name: &str) -> bool {
        false
    }
}
