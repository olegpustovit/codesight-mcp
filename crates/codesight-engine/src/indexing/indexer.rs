//! Thin wrapper over `codesight_language_support::LanguageRegistry` used by
//! indexing and handler paths. Replaces the former `codesight-backend` crate:
//! tree-sitter is the only extraction backend and the `CodeBackend` /
//! `BackendRouter` abstraction was aspirational LSP plumbing that never
//! landed.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use codesight_core::StoredSymbol;
use codesight_language_support::{LanguageRegistry, builtin_extractor_version_stamp};

pub struct Indexer {
    registry: Mutex<Option<LanguageRegistry>>,
    workspace_root: Mutex<Option<PathBuf>>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}

impl Indexer {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(None),
            workspace_root: Mutex::new(None),
        }
    }

    fn with_registry<F, R>(&self, project_root: &Path, f: F) -> R
    where
        F: FnOnce(&LanguageRegistry) -> R,
    {
        let mut cached = self.registry.lock().expect("poisoned");
        let mut root_slot = self.workspace_root.lock().expect("poisoned");
        let rebuild = !matches!(
            (&*cached, &*root_slot),
            (Some(_), Some(prev)) if prev == project_root
        );
        if rebuild {
            *cached = Some(
                LanguageRegistry::for_workspace_root(project_root)
                    .unwrap_or_else(|_| LanguageRegistry::builtin()),
            );
            *root_slot = Some(project_root.to_path_buf());
        }
        f(cached.as_ref().expect("initialized above"))
    }

    pub fn detect_language(&self, project_root: &Path, path: &Path) -> Option<String> {
        self.with_registry(project_root, |registry| {
            registry
                .for_path(path)
                .map(|lang| lang.id().as_str().to_string())
        })
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
        self.with_registry(project_root, |registry| {
            let Some(lang) = registry.for_path(file) else {
                return Vec::new();
            };
            lang.extract_symbols(&relative, source).unwrap_or_default()
        })
    }

    pub fn version_stamp(&self) -> String {
        builtin_extractor_version_stamp().to_string()
    }
}
