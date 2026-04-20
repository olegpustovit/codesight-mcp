//! Liveness / reachability axis (LANGUAGES.md §4.4).
//!
//! The single home for "is this code live or not?": entrypoints, test files,
//! fixtures, conventional directories. Phase D absorbs
//! `ENTRYPOINT_BASENAMES`, `is_test_file`, `is_test_fixture`,
//! `CONVENTIONAL_DIRS`, and `CONVENTIONAL_BASENAMES` into per-language folders.

use std::path::Path;

pub trait LivenessPolicy: Send + Sync {
    /// Basenames always treated as entry-point roots for this language
    /// (`main.rs`, `lib.rs`, `build.rs`, `main.py`, `__main__.py`, etc.).
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether this language considers `path` a test file.
    fn is_test_file(&self, _path: &str) -> bool {
        false
    }

    /// Whether `path` is a fixture / golden / snapshot file this language's
    /// reachability analysis should skip entirely.
    fn is_test_fixture(&self, _path: &str) -> bool {
        false
    }

    /// Whether `path` is a conventional non-production directory whose files
    /// should be default-excluded from hotspot ranking (scripts/, examples/,
    /// benches/, …). Generic callers combine every language's answer.
    fn is_conventional_path(&self, _path: &str) -> bool {
        false
    }

    /// Hook for Rust's `FOREIGN_TRAIT_BASES` / `impl <Trait> for X` sweep,
    /// Python's `if __name__ == '__main__':` probe, and other language-specific
    /// live-root discoveries that require file I/O. Scoped by `root`; absent
    /// root means the caller has no filesystem anchor.
    fn additional_live_root_files(
        &self,
        _root: Option<&Path>,
        _all_files: &[String],
    ) -> Vec<String> {
        Vec::new()
    }
}

/// Default: treat nothing as special. Non-source languages use this.
pub struct NoopLiveness;

impl LivenessPolicy for NoopLiveness {}
