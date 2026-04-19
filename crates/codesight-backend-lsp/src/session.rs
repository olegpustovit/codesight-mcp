//! Per-(language, project_root) LSP session. Spawns the configured command,
//! runs the `initialize` handshake, and holds the child open for the
//! process lifetime. Request/response only — no document sync.
//!
//! This scaffold owns the struct + spawn path. The live request pipeline
//! (notifications, outstanding-request map, cancellation) lands when the
//! conformance corpus enables end-to-end testing.

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::config::LanguageSpec;

#[derive(Debug, Error)]
pub enum LspSessionError {
    #[error("spawn LSP server '{command}': {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("request timed out after {millis}ms")]
    Timeout { millis: u64 },
}

pub struct LspSession {
    pub language: String,
    pub project_root: PathBuf,
    pub spec: LanguageSpec,
    child: Arc<Mutex<Option<Child>>>,
}

impl LspSession {
    pub fn new(language: String, project_root: PathBuf, spec: LanguageSpec) -> Self {
        Self {
            language,
            project_root,
            spec,
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn spawn(&self) -> Result<(), LspSessionError> {
        let mut cmd = tokio::process::Command::new(&self.spec.command);
        cmd.args(&self.spec.args)
            .current_dir(&self.project_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = cmd.spawn().map_err(|e| LspSessionError::Spawn {
            command: self.spec.command.clone(),
            source: e,
        })?;
        *self.child.lock().await = Some(child);
        Ok(())
    }
}

impl Drop for LspSession {
    fn drop(&mut self) {
        // Child cleanup happens on Arc drop — tokio's Child kills on drop
        // when `kill_on_drop` is set. We keep the default (detach) so a
        // long-lived LSP survives router churn; explicit `shutdown` on
        // the backend terminates it.
    }
}
