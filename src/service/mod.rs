pub(crate) mod agent_v1;
mod args;
mod code_tools;
pub(crate) mod cross_repo_map;
pub mod dispatch_timeout;
mod doc_tools;
pub(crate) mod file_watcher;
mod helpers;
pub mod savings;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::instrument;

use codesight_storage::storage::vector_index::OwnedVectorRow;

use crate::{
    config::AppConfig,
    extractors::ExtractorRegistry,
    reindex_state::ReindexStateManager,
    service::savings::{Meta, MetaBuilder, SavingsLedger},
};
use helpers::{ToolError, ToolErrorCategory, attach_meta};

/// Unwrap a store-open result, returning an error response on failure.
///
/// Replaces the 4-line `let store = match self.ensure_*_store(...) { Ok(s) => s,
/// Err(e) => return self.error_response(timer, e) };` pattern that appears ~35
/// times across handler modules.
macro_rules! require_store {
    (repo: $self:expr, $source_id:expr, $repo_path:expr, $timer:expr) => {
        match $self.ensure_repo_store($source_id, $repo_path) {
            Ok(s) => s,
            Err(e) => return $self.error_response($timer, e),
        }
    };
    (doc: $self:expr, $source_id:expr, $repo_path:expr, $timer:expr) => {
        match $self.ensure_doc_store($source_id, $repo_path) {
            Ok(s) => s,
            Err(e) => return $self.error_response($timer, e),
        }
    };
}

pub(crate) use require_store;

const MAX_QUERY_LEN: usize = 1000;

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

pub struct CodeService {
    config: AppConfig,
    ledger: SavingsLedger,
    ledger_guard: Mutex<()>,
    reindex_states: Arc<ReindexStateManager>,
    watchers: file_watcher::WatcherManager,
    embedder: crate::retrieval::embedder::EmbedderSlot,
}

impl CodeService {
    pub fn new(config: AppConfig) -> anyhow::Result<Self> {
        config.ensure_layout()?;
        agent_v1::validate::init_schemas()
            .map_err(|e| anyhow::anyhow!("tools.json schema init failed: {e}"))?;
        let ledger = SavingsLedger::new(config.savings_file());
        let reindex_states = Arc::new(ReindexStateManager::new());
        if config.strict_freshness {
            // Honor the global flag; individual handlers can still block on
            // `await_freshness_if_strict` without plumbing the config down.
            if let Err(error) = reindex_states.set_freshness_mode("strict") {
                tracing::warn!(error = %error, "failed to apply strict freshness mode");
            }
        }
        Ok(Self {
            config,
            ledger,
            ledger_guard: Mutex::new(()),
            reindex_states,
            watchers: file_watcher::WatcherManager::new(),
            embedder: crate::retrieval::embedder::EmbedderSlot::new(),
        })
    }

    /// Lazy-init access to the dense embedder. `Err` means the model could
    /// not be loaded in this session — callers translate to
    /// `SEMANTIC_FALLBACK` warnings rather than erroring the request.
    pub fn embedder(&self) -> Result<&std::sync::Arc<crate::retrieval::embedder::Embedder>, &str> {
        self.embedder.get()
    }

    pub fn enable_background_watch(&self) {
        self.watchers.enable();
    }

    /// Run `f` with the savings-ledger mutex held. Poison is recovered
    /// because a poisoned lock only means a previous caller panicked mid-run;
    /// the on-disk JSON is still readable.
    fn with_ledger_locked<R>(&self, f: impl FnOnce(&SavingsLedger) -> R) -> R {
        let _guard = self
            .ledger_guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(&self.ledger)
    }

    fn ledger_success_meta<T: serde::Serialize>(
        &self,
        timer: &MetaBuilder,
        body: &T,
    ) -> anyhow::Result<Meta> {
        self.with_ledger_locked(|ledger| timer.success(body, ledger))
    }

    fn ledger_error_meta(&self, timer: &MetaBuilder) -> anyhow::Result<Meta> {
        self.with_ledger_locked(|ledger| timer.error(ledger))
    }

    /// Expose the shared reindex state manager so external orchestration and
    /// tests can observe or drive lifecycle transitions.
    pub fn reindex_states(&self) -> &Arc<ReindexStateManager> {
        &self.reindex_states
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn encode_chunks(&self, chunks: Vec<(String, String)>) -> Result<Vec<OwnedVectorRow>, String> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let embedder = self.embedder().map_err(str::to_string)?.clone();
        let (stable_ids, texts): (Vec<_>, Vec<_>) = chunks.into_iter().unzip();
        let vectors = embedder.encode(&texts).map_err(|e| e.to_string())?;
        Ok(stable_ids
            .into_iter()
            .zip(vectors)
            .map(|(stable_id, embedding)| OwnedVectorRow {
                stable_id,
                embedding,
            })
            .collect())
    }

    #[instrument(level = "info", skip(self, arguments), fields(tool = tool_name))]
    pub fn dispatch(&self, tool_name: &str, arguments: Value) -> anyhow::Result<Value> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            agent_v1::dispatch(self, tool_name, arguments)
        }));
        let dispatched = match result {
            Ok(inner) => inner,
            Err(panic_payload) => {
                let message = panic_message(&panic_payload);
                tracing::error!(tool = tool_name, %message, "tool handler panicked");
                Err(ToolError::new(
                    ToolErrorCategory::Internal,
                    "HANDLER_PANIC",
                    format!("tool '{tool_name}' panicked: {message}"),
                    json!({ "tool": tool_name }),
                )
                .into())
            }
        };
        match dispatched {
            Ok(value) => Ok(value),
            Err(err) => {
                // Any leaked error becomes a categorized error response so the MCP
                // boundary never returns an uncategorized failure. If the underlying
                // error is a ToolError, preserve its category; otherwise classify as
                // Internal with the full context chain in the message.
                let tool_error = match err.downcast::<ToolError>() {
                    Ok(te) => te,
                    Err(other) => ToolError::new(
                        ToolErrorCategory::Internal,
                        "INTERNAL_ERROR",
                        format!("{other:#}"),
                        json!({ "tool": tool_name }),
                    ),
                };
                let timer = MetaBuilder::new(0);
                self.error_response(&timer, tool_error)
            }
        }
    }

    pub(super) fn extractors_for_root(&self, root: &std::path::Path) -> ExtractorRegistry {
        ExtractorRegistry::for_workspace_root(root)
    }

    pub(super) fn success_response(
        &self,
        timer: &MetaBuilder,
        body: Value,
    ) -> anyhow::Result<Value> {
        self.success_response_with_warnings(timer, body, Vec::new())
    }

    pub(super) fn success_response_with_warnings(
        &self,
        timer: &MetaBuilder,
        data: Value,
        warnings: Vec<Value>,
    ) -> anyhow::Result<Value> {
        self.success_response_with_meta_extra(timer, data, warnings, [])
    }

    pub(super) fn success_response_with_meta_extra(
        &self,
        timer: &MetaBuilder,
        data: Value,
        warnings: Vec<Value>,
        meta_extra: impl IntoIterator<Item = (String, Value)>,
    ) -> anyhow::Result<Value> {
        let body = json!({
            "ok": true,
            "data": data,
            "warnings": warnings,
        });
        let mut meta_val = serde_json::to_value(self.ledger_success_meta(timer, &body)?)?;
        if let Value::Object(ref mut map) = meta_val {
            for (k, v) in meta_extra {
                map.insert(k, v);
            }
        }
        attach_meta(body, meta_val)
    }

    pub(super) fn error_response(
        &self,
        timer: &MetaBuilder,
        error: ToolError,
    ) -> anyhow::Result<Value> {
        let meta = self.ledger_error_meta(timer)?;
        let body = json!({
            "ok": false,
            "data": { "detail": error.detail.clone() },
            "warnings": [],
            "error": {
                "category": error.category,
                "code": error.code,
                "message": error.message,
                "detail": error.detail,
            },
        });
        // Surface the category on `_meta.error_category` so clients can dispatch
        // on category (timeout, invalid_argument, not_found, …) without peeking
        // into the nested error object. Source of truth stays on `error.category`.
        let mut meta_val = serde_json::to_value(meta)?;
        if let Value::Object(ref mut map) = meta_val {
            map.insert(
                "error_category".to_string(),
                serde_json::to_value(error.category)?,
            );
        }
        attach_meta(body, meta_val)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::tempdir;

    use super::CodeService;
    use crate::config::AppConfig;

    #[test]
    fn indexes_searches_and_retrieves_python_symbols() {
        let workspace = tempdir().expect("tempdir");
        let repo_root = workspace.path().join("mini-repo");
        fs::create_dir_all(&repo_root).unwrap();
        fs::write(
            repo_root.join("api.py"),
            r#"
MAX_RETRIES = 5

class SessionManager:
    """Database session dependency container."""

    def get_db(self):
        """Dependency injection database session."""
        return "db"
"#,
        )
        .unwrap();

        let index_root = workspace.path().join("index");
        let service = CodeService::new(AppConfig::new(index_root)).expect("service");

        let indexed = service
            .dispatch(
                "ingest_sources",
                json!({
                    "path": repo_root.to_string_lossy(),
                    "domains": "code",
                    "incremental": true,
                }),
            )
            .expect("indexing");
        assert!(indexed["ok"].as_bool().unwrap());
        let repo = indexed["data"]["source_id"].as_str().unwrap().to_string();

        let searched = service
            .dispatch(
                "search_catalog",
                json!({
                    "source_id": repo,
                    "query": "dependency injection database session",
                    "domain": "code",
                    "limit": 5,
                }),
            )
            .expect("searching");
        assert!(searched["ok"].as_bool().unwrap());
        let first_id = searched["data"]["items"][0]["ref"]["stable_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(first_id, "api.py::SessionManager.get_db#method");

        let retrieved = service
            .dispatch(
                "read_content",
                json!({
                    "source_id": repo,
                    "refs": [{ "kind": "symbol", "stable_id": first_id }],
                }),
            )
            .expect("retrieving");
        assert!(retrieved["ok"].as_bool().unwrap());
        let src = retrieved["data"]["items"][0]["source"].as_str().unwrap();
        assert!(src.contains("Dependency injection database session."));
    }

    #[test]
    fn concurrent_ledger_updates_are_not_lost() {
        // Route N concurrent ledger writes through `with_ledger_locked` and
        // assert the final total is the exact sum. Without the mutex, the
        // read-modify-write inside `SavingsLedger::record` lets two threads
        // load the same base value and clobber each other's increment, so
        // the final total would drift below the expected sum.
        let workspace = tempdir().expect("tempdir");
        let service = Arc::new(
            CodeService::new(AppConfig::new(workspace.path().join("index"))).expect("service"),
        );

        const THREADS: u64 = 32;
        const DELTA: u64 = 1_000;
        let mut handles = Vec::new();
        for _ in 0..THREADS {
            let svc = service.clone();
            handles.push(std::thread::spawn(move || {
                svc.with_ledger_locked(|ledger| ledger.record(DELTA))
                    .expect("record");
            }));
        }
        for h in handles {
            h.join().expect("join");
        }

        let total = service.ledger.current_total().expect("current_total");
        assert_eq!(total, THREADS * DELTA, "ledger lost updates");
    }

    #[test]
    fn ingest_returns_reindex_busy_when_already_in_progress() {
        let workspace = tempdir().expect("tempdir");
        let repo_root = workspace.path().join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        fs::write(repo_root.join("lib.py"), "x = 1\n").unwrap();

        let service =
            CodeService::new(AppConfig::new(workspace.path().join("index"))).expect("service");

        let repo_id = crate::repo_id::normalize_repo_id(&repo_root.canonicalize().unwrap());
        service
            .reindex_states()
            .try_mark_reindex_start(&repo_id)
            .expect("first claim");

        let out = service
            .dispatch(
                "ingest_sources",
                json!({
                    "path": repo_root.to_string_lossy(),
                    "domains": "code",
                    "incremental": true,
                }),
            )
            .expect("dispatch");
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["category"], json!("transient"));
        assert_eq!(out["error"]["code"], json!("REINDEX_BUSY"));
    }
}
