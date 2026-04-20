//! Generic LSP `CodeBackend`. Config-driven: per-language server command +
//! init options.

pub mod config;
pub mod health;
pub mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{
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
