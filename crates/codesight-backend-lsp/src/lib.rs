//! Generic LSP `CodeBackend`. One impl covers rust-analyzer, pyright,
//! ts-server, gopls, clangd — the language table is config-driven. Each
//! `(language, project_root)` pair gets a long-lived `LspSession` that
//! spawns the LSP subprocess once and reuses it for subsequent queries.
//!
//! Architecture:
//! - `config::LspConfig` — per-language spawn spec loaded from
//!   `.codesight.lsp.toml`. Strategy pattern at the config layer; one
//!   codepath for every language.
//! - `health::HealthTracker` — crash-loop gate: three consecutive failures
//!   within 60s mark a language "unhealthy" for the process lifetime.
//! - `LspBackend` — the `CodeBackend` impl. Queries go through
//!   `LspSession::request` with a per-call timeout.
//!
//! Scope for this scaffold: trait wiring, config loader, health gate.
//! Live LSP sessions land in a follow-up — the conformance gate asks
//! both backends to answer the same queries, and that needs the
//! conformance corpus (MEGAPLAN §8 open question).

pub mod config;
pub mod health;
pub mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use codesight_backend::{
    BackendError, CallHierarchy, Capabilities, CodeBackend, FactQualityTier, HoverInfo,
    Language, Location, RenamePlan, Scope, SymbolRef,
};

pub use config::LspConfig;
pub use health::HealthTracker;
pub use session::LspSession;

const BACKEND_NAME: &str = "lsp";

pub struct LspBackend {
    config: LspConfig,
    languages: Vec<Language>,
    sessions: Mutex<HashMap<SessionKey, LspSession>>,
    health: HealthTracker,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    language: String,
    project_root: PathBuf,
}

impl LspBackend {
    pub fn new(config: LspConfig) -> Self {
        let languages = config
            .languages
            .keys()
            .map(|lang| Language::new(lang.clone()))
            .collect();
        Self {
            config,
            languages,
            sessions: Mutex::new(HashMap::new()),
            health: HealthTracker::new(),
        }
    }

    /// Config-driven language detection: extension → language name.
    pub fn language_for(&self, path: &Path) -> Option<String> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        for (lang, spec) in &self.config.languages {
            if spec.file_extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
                return Some(lang.clone());
            }
        }
        None
    }
}

impl CodeBackend for LspBackend {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn capabilities(&self) -> Capabilities {
        // Conservative capabilities — individual methods still gate by
        // `has_cap` before calling through. Language-aware features
        // (rename/refs) are the whole point of LSP.
        Capabilities {
            has_index: false,
            has_symbols_in_file: true,
            has_workspace_symbols: true,
            has_references: true,
            has_definition: true,
            has_rename: true,
            has_call_hierarchy: true,
            has_hover: true,
            has_diagnostics: false,
            fact_quality: FactQualityTier::LspAuthoritative,
        }
    }

    fn languages(&self) -> &[Language] {
        &self.languages
    }

    fn detect_language(&self, _project_root: &Path, path: &Path) -> Option<String> {
        self.language_for(path)
    }

    fn version_stamp(&self) -> String {
        // LSP is a live source — no version stamp. Fingerprint cache
        // invalidation is tree-sitter's job when that backend is also
        // wired; the router decides per-capability which one answers.
        String::new()
    }

    fn symbols_in_file(&self, path: &Path) -> Result<Vec<SymbolRef>, BackendError> {
        let Some(language) = self.language_for(path) else {
            return Err(BackendError::NotSupported {
                backend: BACKEND_NAME,
                capability: "symbols_in_file (no matching language)",
            });
        };
        if !self.health.is_healthy(&language) {
            return Err(BackendError::TrustDenied {
                backend: BACKEND_NAME,
                reason: format!("LSP for {language} marked unhealthy"),
            });
        }
        // Live LSP queries land in a follow-up session once the
        // conformance corpus is in place — see MEGAPLAN §8.
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "symbols_in_file (not yet wired to session)",
        })
    }

    fn workspace_symbols(
        &self,
        _query: &str,
        _scope: Scope,
    ) -> Result<Vec<SymbolRef>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "workspace_symbols (not yet wired)",
        })
    }

    fn references(&self, _sym: &SymbolRef) -> Result<Vec<Location>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "references (not yet wired)",
        })
    }

    fn call_hierarchy(&self, _sym: &SymbolRef) -> Result<CallHierarchy, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "call_hierarchy (not yet wired)",
        })
    }

    fn rename_prepare(
        &self,
        _loc: &Location,
        _new_name: &str,
    ) -> Result<RenamePlan, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "rename_prepare (not yet wired)",
        })
    }

    fn hover(&self, _loc: &Location) -> Result<Option<HoverInfo>, BackendError> {
        Err(BackendError::NotSupported {
            backend: BACKEND_NAME,
            capability: "hover (not yet wired)",
        })
    }

    fn shutdown(&self) -> Result<(), BackendError> {
        let mut sessions = self.sessions.lock().expect("poisoned");
        sessions.clear();
        Ok(())
    }
}
