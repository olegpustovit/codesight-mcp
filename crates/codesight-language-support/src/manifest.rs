//! Manifest types and re-exports for builtin `LanguageId` constants.
//!
//! `LanguageManifest` is the single source of truth for:
//!   * path → language identity (filename overrides, compound exts, exts),
//!   * grammar + builtin query pack,
//!   * host-profile attachment (§3.3 host profiles),
//!   * source-aware content probes.

use std::path::Path;

// `LanguageId` and its builtin constants live in `codesight-core` (L0) so
// pipeline types (`StoredSymbol`, `IndexedFileUpdate`, `FileFingerprint`)
// can hold typed language identity without reaching up into L0.75.
pub use codesight_core::language_id::{
    AL, ANSIBLE, ASM, BASH, BLADE, C, CPP, CSHARP, CSS, DART, EJS, ELIXIR, ERLANG, FORTRAN, GLEAM,
    GO, JAVA, JAVASCRIPT, JSON, JSX, KOTLIN, LESS, LUA, LUAU, LanguageId, MARKDOWN, NIX, OPENAPI,
    PERL, PHP, PYTHON, RAZOR, RUBY, RUST, SASS, SCSS, SQL, STYL, SWIFT, TOML, TREE_SITTER_QUERY,
    TSX, TYPESCRIPT, UNKNOWN_LANGUAGE, VERSE, VUE, XML, YAML,
};

/// Host-profile identifier (e.g. `openapi`, `ansible`, `nextjs`, `dbt`).
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct ProfileId(&'static str);

impl ProfileId {
    pub const fn new(s: &'static str) -> Self {
        Self(s)
    }
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Result of path / source classification. Host + optional attached profile.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct ResolvedLanguage {
    pub host: LanguageId,
    pub profile: Option<ProfileId>,
}

impl ResolvedLanguage {
    pub const fn host_only(host: LanguageId) -> Self {
        Self {
            host,
            profile: None,
        }
    }
    pub const fn with_profile(host: LanguageId, profile: ProfileId) -> Self {
        Self {
            host,
            profile: Some(profile),
        }
    }
    pub const fn unknown() -> Self {
        Self::host_only(UNKNOWN_LANGUAGE)
    }
}

/// Tree-sitter grammar binding. Every curated language manifest that has a
/// grammar references one of the statics declared in `crate::grammars`.
pub struct GrammarSpec {
    pub grammar_name: &'static str,
    pub loader: fn() -> tree_sitter::Language,
    pub tags_scm: &'static str,
    pub injections_scm: Option<&'static str>,
}

impl GrammarSpec {
    /// Call the underlying tree-sitter loader. Small convenience so callers
    /// don't have to write `(spec.loader)()`.
    pub fn language(&self) -> tree_sitter::Language {
        (self.loader)()
    }
}

/// A host-language semantic overlay (OpenAPI / Ansible / Next.js / dbt / ...).
///
/// Profiles are attached *after* the host language is identified — they do
/// not replace it. Multiple host languages may share a profile via
/// `applies_to`, lowest `priority` wins when several match.
#[derive(Copy, Clone)]
pub struct HostProfile {
    pub id: ProfileId,
    pub priority: u16,
    pub matches_path: fn(path: &Path) -> bool,
    pub applies_to: &'static [LanguageId],
}

/// Complete per-language metadata. Replaces `language_for_path`,
/// `language_profile`, and the builtin grammar catalog.
pub struct LanguageManifest {
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub filename_overrides: &'static [&'static str],
    pub compound_extensions: &'static [&'static str],
    pub grammar: Option<&'static GrammarSpec>,
    pub profiles: &'static [HostProfile],
    pub content_probe: Option<fn(path: &Path, source: &str) -> Option<LanguageId>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_id_is_a_typed_newtype_not_a_string_alias() {
        let rust_id: LanguageId = RUST;
        assert_eq!(rust_id.as_str(), "rust");
        assert_ne!(rust_id, TYPESCRIPT);
    }

    #[test]
    fn resolved_language_carries_optional_profile() {
        let host = ResolvedLanguage::host_only(YAML);
        let with = ResolvedLanguage::with_profile(YAML, ProfileId::new("openapi"));
        assert_eq!(host.host, YAML);
        assert!(host.profile.is_none());
        assert_eq!(with.profile.map(|p| p.as_str()), Some("openapi"));
    }

    #[test]
    fn unknown_language_is_distinct_from_every_builtin() {
        for id in [RUST, TYPESCRIPT, YAML, PYTHON, GO] {
            assert_ne!(id, UNKNOWN_LANGUAGE);
        }
    }
}
