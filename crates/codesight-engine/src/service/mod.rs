pub mod agent_v1;
mod args;
mod code_tools;
mod doc_tools;
mod envelope;
pub(crate) mod file_watcher;
pub(crate) mod helpers;
pub mod savings;

use rustc_hash::FxHashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Mutex, OnceLock, PoisonError, RwLock, Weak,
    atomic::{AtomicU64, Ordering},
};

use envelope::{compact_meta, extract_full_file_read_paths, extract_read_content_paths};

/// Attach a `SUGGEST_OUTLINE` warning to a `read_content` response on the
/// 2nd full-file read of the same path in one session, and escalate to a
/// `suggested_call` block on the 3rd. Unhandled entries pass through. Keeps
/// the nudge entirely in-memory — no new persistence beyond the existing
/// `SessionState.full_file_read_counts` map.
fn attach_full_file_read_nudges(response: &mut Value, counts: &[(String, u8)]) {
    if counts.is_empty() {
        return;
    }
    let Value::Object(body) = response else {
        return;
    };
    let warnings = body
        .entry("warnings".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(arr) = warnings else {
        return;
    };
    for (path, n) in counts {
        if *n < 2 {
            continue;
        }
        let mut warning = json!({
            "code": "SUGGEST_OUTLINE",
            "path": path,
            "full_file_reads_this_session": n,
            "message": format!(
                "read '{path}' in full {n}× this session — prefer `inspect_structure view=file_outline` first, then batch symbol refs via `read_content`."
            ),
        });
        if *n >= 3
            && let Value::Object(map) = &mut warning
        {
            map.insert(
                "suggested_call".to_string(),
                json!({
                    "tool": "inspect_structure",
                    "args": { "view": "file_outline", "file_path": path },
                }),
            );
        }
        arr.push(warning);
    }
}
use serde_json::{Value, json};
use tracing::instrument;

use codesight_analysis::complexity::cyclomatic_complexity_for_path;
use codesight_storage::storage::{QualitySnapshot, vector_index::OwnedVectorRow};

use crate::indexing::Indexer;

use crate::{
    config::{
        AppConfig, BUILD_SHA, BUILD_TIME, BUILD_VERSION, RETRIEVAL_ENGINE, RETRIEVAL_VERSION,
    },
    reindex_state::ReindexStateManager,
    service::savings::{
        Meta, MetaBuilder, QualityHotspotMarker, QualitySnapshotMarker, SavingsLedger, SessionState,
    },
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

use crate::server_support::agent_readiness_cache;

pub struct CodeService {
    config: AppConfig,
    ledger: SavingsLedger,
    ledger_guard: Mutex<()>,
    reindex_states: Arc<ReindexStateManager>,
    watchers: file_watcher::WatcherManager,
    embedder: codesight_retrieval::embedder::EmbedderSlot,
    /// Process-wide per-source snapshot cache used by cross-source paths
    /// (cross-repo `inspect_graph`, `catalog_sources`). Each loop body that
    /// used to reopen an SQLite handle and run a handful of cheap queries
    /// now takes a `RwLock::read` + fingerprint compare on the warm path.
    source_registry: Arc<crate::server_support::source_registry::SourceRegistry>,
    /// Weak back-reference installed by [`CodeService::install_arc_handle`].
    /// Populated on the production path (Arc-wrapped server, watchers, hooks)
    /// so handlers can spawn background work that upgrades the pointer and
    /// calls back into the service. Uninstalled in unit tests that drive
    /// `CodeService` directly by value — those take the synchronous fallback.
    self_weak: OnceLock<Weak<CodeService>>,
    /// Tree-sitter-backed indexer. Wraps a `LanguageRegistry` and provides
    /// language detection + symbol extraction. Always present — handlers
    /// never conditionally probe for a backend.
    indexer: Arc<Indexer>,
    /// Monotonic generation counter bumped every time an ingest commits or
    /// `manage_indexes action ∈ {invalidate, delete, rebuild}` runs. The
    /// `catalog_sources` handler uses it to memoise its JSON body.
    catalog_generation: AtomicU64,
    /// Per-process `catalog_sources` body cache. Stored as
    /// `(generation_at_build, body)`. Not persisted — memoisation across the
    /// lifetime of one server instance only.
    catalog_cache: RwLock<Option<(u64, Value)>>,
}

impl CodeService {
    pub fn new(config: AppConfig) -> anyhow::Result<Self> {
        config.ensure_layout()?;
        agent_v1::validate::init_schemas()
            .map_err(|e| anyhow::anyhow!("tools.json schema init failed: {e}"))?;
        let ledger = SavingsLedger::new(config.savings_file());
        let reindex_states = Arc::new(ReindexStateManager::new());
        Ok(Self {
            indexer: Arc::new(Indexer::new()),
            config,
            ledger,
            ledger_guard: Mutex::new(()),
            reindex_states,
            watchers: file_watcher::WatcherManager::new(),
            embedder: codesight_retrieval::embedder::EmbedderSlot::new(),
            source_registry: Arc::new(crate::server_support::source_registry::SourceRegistry::new()),
            self_weak: OnceLock::new(),
            catalog_generation: AtomicU64::new(0),
            catalog_cache: RwLock::new(None),
        })
    }

    /// Current `catalog_sources` cache generation. Handlers compare this to
    /// the stored `(generation, body)` pair — on match, they can reuse the
    /// cached JSON; on mismatch, they rebuild.
    pub(crate) fn catalog_generation(&self) -> u64 {
        self.catalog_generation.load(Ordering::Acquire)
    }

    /// Bump the catalog generation. Called by any path that changes what
    /// `catalog_sources` would return — ingest success,
    /// `manage_indexes action ∈ {invalidate, delete, rebuild}`.
    pub(crate) fn bump_catalog_generation(&self) {
        self.catalog_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Read the cached body if it still matches the current generation.
    pub(crate) fn cached_catalog(&self) -> Option<Value> {
        let gen_now = self.catalog_generation();
        let guard = self.catalog_cache.read().ok()?;
        match guard.as_ref() {
            Some((g, body)) if *g == gen_now => Some(body.clone()),
            _ => None,
        }
    }

    /// Store a freshly built body under the current generation. Callers pass
    /// the generation they observed *before* building so that a concurrent
    /// bump invalidates them automatically on the next read.
    pub(crate) fn store_catalog(&self, observed_generation: u64, body: Value) {
        if let Ok(mut guard) = self.catalog_cache.write() {
            *guard = Some((observed_generation, body));
        }
    }

    /// Process-wide per-source snapshot cache. Accessible so handler modules
    /// can read or invalidate entries; callers outside the crate should stick
    /// to the typed methods on [`SourceRegistry`].
    pub(crate) fn source_registry(
        &self,
    ) -> &crate::server_support::source_registry::SourceRegistry {
        &self.source_registry
    }

    /// Install the back-pointer so handlers can dispatch background work that
    /// outlives the current request. Call once after wrapping the service in
    /// `Arc`; subsequent calls are no-ops.
    pub fn install_arc_handle(self: &Arc<Self>) {
        let _ = self.self_weak.set(Arc::downgrade(self));
    }

    /// Upgrade the installed weak pointer, if any. Returns `None` for bare
    /// (non-Arc) services, which signals callers to take the synchronous
    /// fallback rather than spawn.
    pub(crate) fn shared_handle(&self) -> Option<Arc<Self>> {
        self.self_weak.get().and_then(Weak::upgrade)
    }

    /// Lazy-init access to the dense embedder. `Err` means the model could
    /// not be loaded in this session — callers translate to
    /// `SEMANTIC_FALLBACK` warnings rather than erroring the request.
    pub fn embedder(
        &self,
    ) -> Result<&std::sync::Arc<codesight_retrieval::embedder::Embedder>, &str> {
        self.embedder.get()
    }

    /// Run `f` with the savings-ledger mutex held. Poison is recovered
    /// because a poisoned lock only means a previous caller panicked mid-run;
    /// the on-disk JSON is still readable.
    pub(crate) fn savings_ledger_current_session(
        &self,
    ) -> std::io::Result<crate::service::savings::SessionState> {
        self.with_ledger_locked(|ledger| ledger.current_session())
    }

    pub(crate) fn savings_ledger_end_session(
        &self,
    ) -> std::io::Result<crate::service::savings::SessionState> {
        self.with_ledger_locked(|ledger| ledger.end_session())
    }

    pub(crate) fn savings_ledger_record_stale_warning(
        &self,
        repo: &str,
        files_in_flight: &[String],
    ) {
        self.with_ledger_locked(|ledger| {
            let _ = ledger.record_stale_warning(repo, files_in_flight);
        });
    }

    pub(crate) fn savings_ledger_set_quality_session_start(&self, marker: QualitySnapshotMarker) {
        self.with_ledger_locked(|ledger| {
            let _ = ledger.set_quality_session_start(marker);
        });
    }

    pub(crate) fn savings_ledger_set_quality_session_end(&self, marker: QualitySnapshotMarker) {
        self.with_ledger_locked(|ledger| {
            let _ = ledger.set_quality_session_end(marker);
        });
    }

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

    fn build_quality_snapshot(
        &self,
        store: &codesight_storage::storage::IndexStore,
        source_id: &str,
        snapshot_name: &str,
        gov: &codesight_analysis::quality_signal::GovernanceSignal,
    ) -> anyhow::Result<(QualitySnapshot, QualitySnapshotMarker)> {
        let (avg_complexity, hotspots) = self.quality_snapshot_metrics(store)?;
        let snapshot = QualitySnapshot {
            source_id: source_id.to_string(),
            name: snapshot_name.to_string(),
            created_at: String::new(),
            score: gov.score,
            structural_score: gov.structural_score,
            agent_readiness_score: gov.agent_readiness_score,
            modularity: gov.structural.root_causes.modularity,
            acyclicity: Some(gov.structural.root_causes.acyclicity),
            depth: gov.structural.root_causes.depth,
            equality: Some(gov.structural.root_causes.equality),
            redundancy: Some(gov.structural.root_causes.redundancy),
            navigability: Some(gov.agent.dimensions.navigability),
            comprehensibility: Some(gov.agent.dimensions.comprehensibility),
            verifiability: Some(gov.agent.dimensions.verifiability),
            change_safety: Some(gov.agent.dimensions.change_safety),
            cycle_count: gov.structural.coverage.cycle_count as u32,
            dead_count: gov.structural.coverage.dead_files as u32,
            dup_count: gov.structural.coverage.duplicate_functions as u32,
            file_count: gov.structural.coverage.graph_files as u32,
            symbol_count: gov.structural.coverage.functions_with_source as u32,
            confidence: gov.confidence.to_string(),
            avg_complexity,
        };
        let marker = QualitySnapshotMarker {
            source_id: source_id.to_string(),
            snapshot_name: snapshot_name.to_string(),
            avg_complexity: Some(avg_complexity),
            hotspots,
        };
        Ok((snapshot, marker))
    }

    fn quality_snapshot_metrics(
        &self,
        store: &codesight_storage::storage::IndexStore,
    ) -> anyhow::Result<(f64, Vec<QualityHotspotMarker>)> {
        let mut file_hotspots: FxHashMap<String, u64> = FxHashMap::default();
        let mut total_complexity = 0_u64;
        let mut function_count = 0_u64;

        for symbol in store.all_symbols_for_analysis()? {
            let kind = symbol.kind.to_ascii_lowercase();
            if kind != "function" && kind != "method" {
                continue;
            }
            let cyclomatic =
                cyclomatic_complexity_for_path(&symbol.source_text, &symbol.file_path) as u64;
            total_complexity = total_complexity.saturating_add(cyclomatic);
            function_count = function_count.saturating_add(1);
            let entry = file_hotspots.entry(symbol.file_path).or_default();
            *entry = (*entry).max(cyclomatic);
        }

        let avg_complexity = if function_count == 0 {
            0.0
        } else {
            total_complexity as f64 / function_count as f64
        };

        let mut hotspots: Vec<QualityHotspotMarker> = file_hotspots
            .into_iter()
            .map(|(file, cyclomatic)| QualityHotspotMarker { file, cyclomatic })
            .collect();
        hotspots.sort_by(|left, right| {
            right
                .cyclomatic
                .cmp(&left.cyclomatic)
                .then_with(|| left.file.cmp(&right.file))
        });
        hotspots.truncate(10);

        Ok((((avg_complexity * 100.0).round()) / 100.0, hotspots))
    }

    fn maybe_capture_session_start_snapshot(&self, source_id: Option<&str>) {
        let Some(source_id) = source_id.filter(|value| !value.is_empty()) else {
            return;
        };
        let should_capture = self
            .savings_ledger_current_session()
            .map(|session| session.quality_session_start.is_none())
            .unwrap_or(false);
        if !should_capture {
            return;
        }
        let Ok(store) = self.ensure_repo_store(source_id, None) else {
            return;
        };
        let Ok(gov) = codesight_analysis::quality_signal::compute_governance_signal(&store) else {
            return;
        };
        let Ok((snapshot, marker)) =
            self.build_quality_snapshot(&store, source_id, "session_start", &gov)
        else {
            return;
        };
        if store.save_snapshot(&snapshot).is_ok() {
            self.savings_ledger_set_quality_session_start(marker);
        }
    }

    fn maybe_capture_session_start_from_catalog(&self, response: &Value, session: &SessionState) {
        if session.quality_session_start.is_some() {
            return;
        }
        let Some(sources) = response
            .get("data")
            .and_then(|data| data.get("sources"))
            .and_then(Value::as_array)
        else {
            return;
        };

        let cwd = std::env::current_dir()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .map(|path| path.to_string_lossy().to_string());

        let code_sources: Vec<&Value> = sources
            .iter()
            .filter(|source| source.get("domain").and_then(Value::as_str) == Some("code"))
            .collect();
        let preferred = code_sources.iter().find_map(|source| {
            let root_hint = source.get("root_hint").and_then(Value::as_str)?;
            let source_id = source.get("source_id").and_then(Value::as_str)?;
            let cwd = cwd.as_deref()?;
            if root_hint == cwd {
                Some(source_id)
            } else {
                None
            }
        });
        let inferred = preferred.or_else(|| {
            if code_sources.len() == 1 {
                code_sources[0].get("source_id").and_then(Value::as_str)
            } else {
                None
            }
        });
        self.maybe_capture_session_start_snapshot(inferred);
    }

    fn session_payload(&self, session: &SessionState) -> Value {
        json!({
            "id": session.id,
            "started_at": session.started_at,
            "ended_at": session.ended_at,
            "tokens_saved_total": session.tokens_saved_total,
            "build_sha": BUILD_SHA,
            "build_time": BUILD_TIME,
            "build_version": BUILD_VERSION,
            "retrieval_engine": RETRIEVAL_ENGINE,
            "retrieval_version": RETRIEVAL_VERSION,
        })
    }

    fn attach_session_and_compact_meta(
        &self,
        tool_name: &str,
        mut value: Value,
        meta_verbose: bool,
    ) -> anyhow::Result<Value> {
        let session = if tool_name == "catalog_sources" {
            let session = self.savings_ledger_current_session()?;
            self.maybe_capture_session_start_from_catalog(&value, &session);
            Some(self.savings_ledger_current_session()?)
        } else {
            None
        };
        if let Some(meta) = value.get_mut("_meta").and_then(Value::as_object_mut)
            && !meta_verbose
        {
            compact_meta(meta);
        }
        if let Some(session) = session
            && let Value::Object(ref mut body) = value
        {
            body.insert("session".to_string(), self.session_payload(&session));
        }
        Ok(value)
    }

    /// Expose the shared reindex state manager so external orchestration and
    /// tests can observe or drive lifecycle transitions.
    pub fn reindex_states(&self) -> &Arc<ReindexStateManager> {
        &self.reindex_states
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub(crate) fn record_agent_readiness_probe(
        &self,
        path: &str,
        score: f64,
        weakest_dimension: &str,
        threshold: f64,
    ) {
        let cache_path = self
            .config
            .index_root()
            .join(agent_readiness_cache::FILE_NAME);
        let mut state = std::fs::read(&cache_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<agent_readiness_cache::State>(&bytes).ok())
            .unwrap_or_default();
        state.entries.insert(
            path.to_string(),
            agent_readiness_cache::Entry {
                score,
                weakest_dimension: weakest_dimension.to_string(),
                threshold,
                computed_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&state) {
            let tmp = cache_path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(tmp, cache_path);
            }
        }
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
        let meta_verbose = match arguments.get("_meta_verbose") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
            _ => false,
        };
        let non_empty_str = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let source_id = non_empty_str("source_id");
        let impact_analysis = (tool_name == "evaluate_impact")
            .then(|| non_empty_str("analysis"))
            .flatten();
        // Consolidated-tool variant: `inspect_graph graph=…`,
        // `inspect_structure view=…`, `evaluate_quality analysis=…`,
        // `evaluate_impact analysis=…`. Per-variant telemetry keys the
        // ledger at `"tool:variant"` alongside the aggregate tool row.
        let variant = match tool_name {
            "inspect_graph" => non_empty_str("graph"),
            "inspect_structure" => non_empty_str("view"),
            "evaluate_quality" | "evaluate_impact" => non_empty_str("analysis"),
            _ => None,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            agent_v1::dispatch(self, tool_name, arguments)
        }));
        let dispatched = match result {
            Ok(inner) => inner,
            Err(panic_payload) => {
                let message = if let Some(s) = panic_payload.downcast_ref::<&'static str>() {
                    (*s).to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "non-string panic payload".to_string()
                };
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
            Ok(mut value) => {
                let tokens_saved = value
                    .pointer("/_meta/tokens_saved")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let timing_ms = value
                    .pointer("/_meta/timing_ms")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let ok = value["ok"].as_bool().unwrap_or(false);
                self.with_ledger_locked(|ledger| {
                    let _ = ledger.record_tool_call(savings::ToolCallRecord {
                        tool: tool_name,
                        tokens_saved,
                        timing_ms,
                        ok,
                        source_id: source_id.as_deref(),
                        impact_analysis: impact_analysis.as_deref(),
                        variant: variant.as_deref(),
                    });
                    // Record read_content-specific detail when present.
                    if tool_name == "read_content"
                        && let Some(scopes) = value.pointer("/_meta/read_scopes")
                    {
                        let detail = savings::ReadContentStats {
                            symbol_reads: scopes["symbol"].as_u64().unwrap_or(0),
                            section_reads: scopes["section"].as_u64().unwrap_or(0),
                            file_range_reads: scopes["file_range"].as_u64().unwrap_or(0),
                            file_full_reads: scopes["file_full"].as_u64().unwrap_or(0),
                            warning_count: value
                                .pointer("/_meta/warned_full_file_reads")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            bytes_returned: value
                                .pointer("/_meta/returned_bytes")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                        };
                        let _ = ledger.record_read_content_detail(detail);
                        let paths = extract_read_content_paths(&value);
                        if !paths.is_empty() {
                            let _ = ledger.record_codesight_read_paths(&paths);
                        }
                    }
                });
                // Repeated full-file read nudge. We bump per-path counters
                // outside the ledger_locked closure because the closure can
                // only consume one `&SavingsLedger` at a time and we want
                // the warning attached to the response before it flows to
                // the envelope-compacting step.
                if tool_name == "read_content" {
                    let full_paths = extract_full_file_read_paths(&value);
                    if !full_paths.is_empty() {
                        let counts = self
                            .with_ledger_locked(|ledger| ledger.record_full_file_reads(&full_paths))
                            .unwrap_or_default();
                        attach_full_file_read_nudges(&mut value, &counts);
                    }
                }
                self.maybe_capture_session_start_snapshot(source_id.as_deref());
                self.attach_session_and_compact_meta(tool_name, value, meta_verbose)
            }
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
                let value = self.error_response(&timer, tool_error)?;
                self.attach_session_and_compact_meta(tool_name, value, meta_verbose)
            }
        }
    }

    /// Tree-sitter-backed indexer. Always available.
    pub(crate) fn indexer(&self) -> &Arc<Indexer> {
        &self.indexer
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
        let first_id = searched["data"]["items"][0]["ref"]["id"]
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
            .try_mark_reindex_start(&repo_id, &[])
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
