//! Package / manifest axis (LANGUAGES.md §4.3).
//!
//! Owns: package-name discovery for a directory, workspace-member discovery
//! from a project root, root-package extraction from an import specifier,
//! and framework-contributed workspace membership when applicable.
//!
//! Phase D.5 exposes `root_package_from_specifier` so the registry is the
//! single source of truth for the "which crate/package owns this import?"
//! question that used to live as a `match language.to_ascii_lowercase()` in
//! `codesight-imports::packages`.

use std::path::Path;

pub trait PackageManifest: Send + Sync {
    /// Package name for the directory this file lives in, if any.
    fn package_name_for_dir(&self, _dir: &Path) -> Option<String> {
        None
    }

    /// Workspace members reachable from this root (empty by default).
    fn workspace_members(&self, _root: &Path) -> Vec<String> {
        Vec::new()
    }

    /// Root-package extraction from an import specifier.
    ///
    /// Default returns the specifier unchanged. Languages with package-path
    /// grammars (Rust `::`, Python `.`, JS scoped packages, Go vendor
    /// paths) override to strip to the root package name. Relative
    /// specifiers (leading `.`) return an empty string to mark them as
    /// non-package references.
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        specifier.to_string()
    }
}

/// Default: no packaging model. Used by languages with no manifest concept
/// (shell, plain YAML, etc.).
pub struct NoopPackages;

impl PackageManifest for NoopPackages {}
