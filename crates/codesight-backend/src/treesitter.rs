//! Tree-sitter `CodeBackend`: wraps `codesight-extractors::ExtractorRegistry`
//! and exposes the subset of capabilities the AST-based pipeline can offer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::{
    BackendError, BackendRouter, CallHierarchy, Capabilities, CodeBackend, FactQualityTier,
    HoverInfo, IndexFileRequest, IndexProjectRequest, IndexSink, IndexStats, IndexedFile,
    Language, Location, RenamePlan, Scope, SymbolRef,
};
use codesight_extractors::extractors::ExtractorRegistry;

const BACKEND_NAME: &str = "tree-sitter";

/// Tree-sitter backend built on top of the shared `ExtractorRegistry`.
pub struct TreeSitterBackend {
    registry: Mutex<Option<ExtractorRegistry>>,
    workspace_root: Mutex<Option<PathBuf>>,
    languages: Vec<Language>,
}

impl TreeSitterBackend {
    /// Build a backend rooted at a future workspace. The registry is built
    /// lazily on first use — matches the historical `ExtractorRegistry::for_workspace_root`
    /// behaviour.
    pub fn new() -> Self {
        let registry = ExtractorRegistry::default();
        let languages = registry
            .known_languages()
            .into_iter()
            .map(|name| Language::new(name.to_string()))
            .collect();
        Self {
            registry: Mutex::new(None),
            workspace_root: Mutex::new(None),
            languages,
        }
    }

    fn registry_for(&self, project_root: &Path) -> ExtractorRegistry {
        let mut cached = self.registry.lock().expect("poisoned");
        let mut root_slot = self.workspace_root.lock().expect("poisoned");
        let rebuild = !matches!(
            (&*cached, &*root_slot),
            (Some(_), Some(prev)) if prev == project_root
        );
        if rebuild {
            *cached = Some(ExtractorRegistry::for_workspace_root(project_root));
            *root_slot = Some(project_root.to_path_buf());
        }
        cached.as_ref().expect("initialized above").clone()
    }
}

impl Default for TreeSitterBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeBackend for TreeSitterBackend {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            has_index: true,
            has_symbols_in_file: true,
            has_workspace_symbols: false,
            has_references: false,
            has_definition: false,
            has_rename: false,
            has_call_hierarchy: false,
            has_hover: false,
            has_diagnostics: false,
            fact_quality: FactQualityTier::TreeSitter,
        }
    }

    fn languages(&self) -> &[Language] {
        &self.languages
    }

    fn detect_language(&self, project_root: &Path, path: &Path) -> Option<String> {
        let registry = self.registry_for(project_root);
        registry
            .extractor_for_path(path)
            .map(|ex| ex.language_name().to_string())
    }

    fn version_stamp(&self) -> String {
        codesight_extractors::extractors::engine::catalogs::builtin_extractor_version_stamp()
            .to_string()
    }

    fn index_file(&self, req: IndexFileRequest) -> Result<IndexedFile, BackendError> {
        let registry = self.registry_for(&req.project_root);
        let Some(extractor) = registry.extractor_for_path(&req.file) else {
            return Ok(IndexedFile {
                file: req.file,
                symbols: Vec::new(),
            });
        };
        let relative = req
            .file
            .strip_prefix(&req.project_root)
            .unwrap_or(&req.file)
            .to_string_lossy()
            .to_string();
        let symbols = extractor
            .extract(&relative, &req.source)
            .map_err(|e| BackendError::internal(BACKEND_NAME, anyhow::anyhow!("{e}")))?;
        Ok(IndexedFile { file: req.file, symbols })
    }

    fn symbols_in_file(&self, path: &Path) -> Result<Vec<SymbolRef>, BackendError> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| BackendError::internal(BACKEND_NAME, anyhow::anyhow!("{e}")))?;
        let project_root = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let indexed = self.index_file(IndexFileRequest {
            project_root: project_root.clone(),
            file: path.to_path_buf(),
            source,
        })?;
        Ok(indexed
            .symbols
            .into_iter()
            .map(|sym| {
                let stable_id = format!(
                    "{}::{}#{:?}",
                    path.display(),
                    sym.name,
                    sym.kind
                );
                SymbolRef {
                    stable_id,
                    file: path.to_path_buf(),
                    name: sym.name,
                }
            })
            .collect())
    }

    fn workspace_symbols(
        &self,
        _query: &str,
        _scope: Scope,
    ) -> Result<Vec<SymbolRef>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "workspace_symbols",
        })
    }
    fn references(&self, _sym: &SymbolRef) -> Result<Vec<Location>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "references",
        })
    }
    fn call_hierarchy(&self, _sym: &SymbolRef) -> Result<CallHierarchy, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "call_hierarchy",
        })
    }
    fn rename_prepare(
        &self,
        _loc: &Location,
        _new_name: &str,
    ) -> Result<RenamePlan, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "rename_prepare",
        })
    }
    fn hover(&self, _loc: &Location) -> Result<Option<HoverInfo>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "hover",
        })
    }
    fn index_project(
        &self,
        _req: IndexProjectRequest,
        _sink: &mut dyn IndexSink,
    ) -> Result<IndexStats, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "index_project",
        })
    }
}

/// Router with a tree-sitter backend pre-registered.
pub fn default_router() -> BackendRouter {
    let mut router = BackendRouter::new();
    router.register(Arc::new(TreeSitterBackend::new()));
    router
}
