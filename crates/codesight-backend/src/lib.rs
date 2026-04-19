//! Code-intelligence backend trait. The engine calls into this crate only;
//! concrete backends (tree-sitter, LSP, hybrid) live in sibling crates and
//! are wired by the host binary.
//!
//! Design principles (MEGAPLAN §2.1):
//! 1. Language-scoped — each backend declares which languages it handles.
//! 2. Capability-declared — a `Capabilities` struct up front; handlers inspect
//!    it, never try-and-catch.
//! 3. Provenance on answers — every fact carries a `codesight_core::FactQuality`.
//! 4. No shared mutable state — one `Box<dyn CodeBackend>` owns its deps.
//! 5. Index vs query paths — backends may implement either or both.

use std::path::{Path, PathBuf};

use codesight_core::StoredSymbol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("capability not supported by backend '{backend}': {capability}")]
    NotSupported {
        backend: &'static str,
        capability: &'static str,
    },
    #[error("backend '{backend}' timed out after {millis}ms on {operation}")]
    Timeout {
        backend: &'static str,
        operation: &'static str,
        millis: u64,
    },
    #[error("backend '{backend}' refused unsafe project root: {reason}")]
    TrustDenied {
        backend: &'static str,
        reason: String,
    },
    #[error("backend '{1}' internal error: {0}")]
    Internal(#[source] anyhow::Error, &'static str),
}

impl BackendError {
    pub fn internal(backend: &'static str, err: anyhow::Error) -> Self {
        Self::Internal(err, backend)
    }
}

/// Declared capability set for a backend. Handlers route per (Language,
/// Capability) and never attempt an operation the backend has not claimed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub has_index: bool,
    pub has_symbols_in_file: bool,
    pub has_workspace_symbols: bool,
    pub has_references: bool,
    pub has_definition: bool,
    pub has_rename: bool,
    pub has_call_hierarchy: bool,
    pub has_hover: bool,
    pub has_diagnostics: bool,
    /// Highest-tier `FactQuality` this backend emits.
    pub fact_quality: FactQualityTier,
}

/// Tier marker so routers can compare backend confidence without inspecting
/// every answer. Aligns with `codesight_core::symbols::FactQuality`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactQualityTier {
    #[default]
    Heuristic,
    TreeSitter,
    Hybrid,
    LspAuthoritative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language(pub String);

impl Language {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifier for a symbol inside a project. The backend-internal stable id
/// is opaque to handlers; combine with `file`/`kind` when the handler needs
/// to reason about what the symbol represents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRef {
    pub stable_id: String,
    pub file: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: PathBuf,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub files: u64,
    pub symbols: u64,
    pub skipped_files: u64,
    pub errors: u64,
}

/// Backends that support bulk indexing stream extracted symbols into this
/// sink rather than returning one giant vector. The engine wires a sink that
/// writes into `codesight-storage`.
pub trait IndexSink {
    fn push(&mut self, file: &Path, symbols: Vec<StoredSymbol>);
    fn warn(&mut self, _file: &Path, _message: &str) {}
}

#[derive(Debug, Clone)]
pub struct IndexFileRequest {
    pub project_root: PathBuf,
    pub file: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub file: PathBuf,
    pub symbols: Vec<StoredSymbol>,
}

#[derive(Debug, Clone)]
pub struct IndexProjectRequest {
    pub project_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Scope {
    /// Repo-relative prefix to restrict a workspace_symbols query.
    pub prefix: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CallHierarchy {
    pub callers: Vec<Location>,
    pub callees: Vec<Location>,
}

#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub edits: Vec<Location>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HoverInfo {
    pub markdown: String,
}

/// The trait. Keep it narrow: only add methods that a concrete handler needs.
pub trait CodeBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn languages(&self) -> &[Language];

    /// Language this backend would assign to `path` when rooted at
    /// `project_root`. `None` means the backend cannot index the file.
    /// Used by the walker/watcher before touching disk.
    fn detect_language(&self, _project_root: &Path, _path: &Path) -> Option<String> {
        None
    }

    /// Opaque string that changes when this backend's extraction logic or
    /// grammar state changes. The engine uses it to invalidate cached
    /// file fingerprints after upgrades. Empty string = unversioned.
    fn version_stamp(&self) -> String {
        String::new()
    }

    // ── Index (optional; pure-query backends return NotSupported) ──────
    fn index_file(&self, _req: IndexFileRequest) -> Result<IndexedFile, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "index_file",
        })
    }
    fn index_project(
        &self,
        _req: IndexProjectRequest,
        _sink: &mut dyn IndexSink,
    ) -> Result<IndexStats, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "index_project",
        })
    }

    // ── Query (each method independently optional) ──────────────────────
    fn symbols_in_file(&self, _path: &Path) -> Result<Vec<SymbolRef>, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "symbols_in_file",
        })
    }
    fn workspace_symbols(
        &self,
        _query: &str,
        _scope: Scope,
    ) -> Result<Vec<SymbolRef>, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "workspace_symbols",
        })
    }
    fn references(&self, _sym: &SymbolRef) -> Result<Vec<Location>, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "references",
        })
    }
    fn definition(&self, _loc: &Location) -> Result<Option<Location>, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "definition",
        })
    }
    fn call_hierarchy(&self, _sym: &SymbolRef) -> Result<CallHierarchy, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "call_hierarchy",
        })
    }
    fn rename_prepare(
        &self,
        _loc: &Location,
        _new_name: &str,
    ) -> Result<RenamePlan, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "rename_prepare",
        })
    }
    fn hover(&self, _loc: &Location) -> Result<Option<HoverInfo>, BackendError> {
        Err(BackendError::NotSupported {
            backend: self.name(),
            capability: "hover",
        })
    }

    // ── Lifecycle ──────────────────────────────────────────────────────
    fn warm(&self, _project_root: &Path) -> Result<(), BackendError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), BackendError> {
        Ok(())
    }
}

/// Per-language routing table. Picks the highest-`FactQuality` backend that
/// claims the requested capability for the file's language. Deterministic
/// tiebreak: registration order.
///
/// MEGAPLAN §2.4 goal: engine dispatch never knows whether a fact came from
/// tree-sitter or LSP — it asks the router for a capable backend.
pub struct BackendRouter {
    backends: Vec<std::sync::Arc<dyn CodeBackend>>,
}

impl Default for BackendRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRouter {
    pub fn new() -> Self {
        Self { backends: Vec::new() }
    }

    pub fn register(&mut self, backend: std::sync::Arc<dyn CodeBackend>) {
        self.backends.push(backend);
    }

    pub fn backends(&self) -> &[std::sync::Arc<dyn CodeBackend>] {
        &self.backends
    }

    /// Pick the highest-tier backend that supports the given capability.
    /// Callers pass a getter that reads the relevant `Capabilities` bit —
    /// keeps the router agnostic about which capability matters.
    pub fn pick<F>(&self, has_cap: F) -> Option<&std::sync::Arc<dyn CodeBackend>>
    where
        F: Fn(&Capabilities) -> bool,
    {
        self.backends
            .iter()
            .filter(|b| has_cap(&b.capabilities()))
            .max_by_key(|b| b.capabilities().fact_quality)
    }

    /// Convenience: the indexer backend (the one with `has_index=true`).
    /// Used by engine indexing paths.
    pub fn indexer(&self) -> Option<&std::sync::Arc<dyn CodeBackend>> {
        self.pick(|c| c.has_index)
    }
}
