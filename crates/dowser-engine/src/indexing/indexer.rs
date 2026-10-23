//! Thin wrapper over `dowser_language_support::LanguageRegistry` used by
//! indexing and handler paths.
//!
//! The registry is immutable after construction and `Send + Sync`, so we hand
//! it out as `Arc<LanguageRegistry>` and callers read it lock-free. Parallel
//! ingest workers each clone the `Arc` once and then hit `for_path` /
//! `extract_symbols` without touching a mutex.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dowser_core::StoredSymbol;
use dowser_language_support::{LanguageRegistry, builtin_extractor_version_stamp};

pub struct Indexer {
    /// Cached `(workspace_root, registry)` pair. The mutex is only touched on
    /// the cold path — first look-up for a given root. Hot lookups clone the
    /// `Arc<LanguageRegistry>` and return without holding the lock.
    cache: Mutex<Option<(PathBuf, Arc<LanguageRegistry>)>>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}

impl Indexer {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(None),
        }
    }

    /// Return a shareable `Arc<LanguageRegistry>` for `project_root`. Workers
    /// pass this around freely; the registry itself is read-only and fully
    /// `Send + Sync`.
    pub fn registry_for(&self, project_root: &Path) -> Arc<LanguageRegistry> {
        let mut guard = self.cache.lock().expect("poisoned");
        if let Some((prev, reg)) = guard.as_ref()
            && prev == project_root
        {
            return Arc::clone(reg);
        }
        let reg = Arc::new(
            LanguageRegistry::for_workspace_root(project_root)
                .unwrap_or_else(|_| LanguageRegistry::builtin()),
        );
        *guard = Some((project_root.to_path_buf(), Arc::clone(&reg)));
        reg
    }

    pub fn detect_language(&self, project_root: &Path, path: &Path) -> Option<String> {
        self.registry_for(project_root)
            .for_path(path)
            .map(|lang| lang.id().as_str().to_string())
    }

    pub fn extract_symbols(
        &self,
        project_root: &Path,
        file: &Path,
        source: &str,
    ) -> Vec<StoredSymbol> {
        let relative = file
            .strip_prefix(project_root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        let registry = self.registry_for(project_root);
        let Some(lang) = registry.for_path(file) else {
            return Vec::new();
        };
        lang.extract_symbols(&relative, source).unwrap_or_default()
    }

    pub fn version_stamp(&self) -> String {
        builtin_extractor_version_stamp().to_string()
    }
}
