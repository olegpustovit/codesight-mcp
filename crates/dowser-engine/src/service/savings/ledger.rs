//! JSON-file-backed savings ledger. Tracks cumulative token savings,
//! per-session tool counters, and related markers. Writes are atomic
//! (tmp file + rename) and reads tolerate missing files.

use std::{fs, io, path::PathBuf};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::pricing::{cost_for_tokens, round_cost, round_cost_map};
use super::session::{QualitySnapshotMarker, ReadContentStats, SessionState, StaleWarningRecord};

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavingsState {
    total_tokens_saved: u64,
    #[serde(default)]
    total_cost_avoided: FxHashMap<String, f64>,
    #[serde(default)]
    current_session: SessionState,
}

#[derive(Debug, Clone)]
pub struct SavingsLedger {
    path: PathBuf,
}

/// Input for [`SavingsLedger::record_tool_call`].
#[derive(Debug, Clone, Copy)]
pub struct ToolCallRecord<'a> {
    pub tool: &'a str,
    pub tokens_saved: u64,
    pub timing_ms: f64,
    pub ok: bool,
    pub source_id: Option<&'a str>,
    pub impact_analysis: Option<&'a str>,
    pub variant: Option<&'a str>,
}

impl SavingsLedger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn current_total(&self) -> io::Result<u64> {
        Ok(self.load()?.total_tokens_saved)
    }

    pub fn record(&self, delta: u64) -> io::Result<(u64, FxHashMap<String, f64>)> {
        let mut state = self.load()?;
        state.total_tokens_saved = state.total_tokens_saved.saturating_add(delta);
        for (model, cost) in cost_for_tokens(delta) {
            let entry = state.total_cost_avoided.entry(model).or_default();
            *entry = round_cost(*entry + cost);
        }
        let total = state.total_tokens_saved;
        let total_cost = round_cost_map(state.total_cost_avoided.clone());
        self.store(&state)?;
        Ok((total, total_cost))
    }

    pub fn current_state(&self) -> io::Result<(u64, FxHashMap<String, f64>)> {
        let state = self.load()?;
        Ok((
            state.total_tokens_saved,
            round_cost_map(state.total_cost_avoided),
        ))
    }

    pub fn ensure_session(&self) -> io::Result<(SessionState, bool)> {
        let mut state = self.load()?;
        let created = ensure_active_session(&mut state);
        let session = state.current_session.clone();
        if created {
            self.store(&state)?;
        }
        Ok((session, created))
    }

    pub fn record_tool_call(&self, call: ToolCallRecord<'_>) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let session = &mut state.current_session;
        if session.primary_source_id.is_none()
            && let Some(source_id) = call.source_id.filter(|value| !value.is_empty())
        {
            session.primary_source_id = Some(source_id.to_string());
        }
        session.tokens_saved_total = session.tokens_saved_total.saturating_add(call.tokens_saved);
        if call.tool == "impact"
            && let Some(analysis) = call.impact_analysis.filter(|value| !value.is_empty())
        {
            *session
                .impact_usage
                .entry(analysis.to_string())
                .or_default() += 1;
        }
        {
            let entry = session.per_tool.entry(call.tool.to_string()).or_default();
            entry.calls = entry.calls.saturating_add(1);
            entry.tokens_saved = entry.tokens_saved.saturating_add(call.tokens_saved);
            entry.timing_ms_sum += call.timing_ms;
            if !call.ok {
                entry.error_count = entry.error_count.saturating_add(1);
            }
        }
        // Per-variant record for consolidated tools (`inspect_graph graph=X`,
        // `inspect_structure view=X`, `evaluate_quality analysis=X`,
        // `evaluate_impact analysis=X`). We emit a second row keyed
        // `"tool:variant"` so `session_stats` can answer "which enum value
        // failed?" without a parallel schema. The aggregate row is still
        // updated above so top-level roll-ups are unchanged.
        if let Some(variant) = call.variant.filter(|value| !value.is_empty()) {
            let key = format!("{tool}:{variant}", tool = call.tool);
            let entry = session.per_tool.entry(key).or_default();
            entry.calls = entry.calls.saturating_add(1);
            entry.tokens_saved = entry.tokens_saved.saturating_add(call.tokens_saved);
            entry.timing_ms_sum += call.timing_ms;
            if !call.ok {
                entry.error_count = entry.error_count.saturating_add(1);
            }
        }
        self.store(&state)
    }

    /// Record read_content-specific detail (scope counters, warnings, bytes).
    pub fn record_read_content_detail(&self, detail: ReadContentStats) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let entry = state
            .current_session
            .per_tool
            .entry("read_content".to_string())
            .or_default();
        let d = entry
            .read_content_detail
            .get_or_insert_with(ReadContentStats::default);
        d.symbol_reads += detail.symbol_reads;
        d.section_reads += detail.section_reads;
        d.file_range_reads += detail.file_range_reads;
        d.file_full_reads += detail.file_full_reads;
        d.warning_count += detail.warning_count;
        d.bytes_returned += detail.bytes_returned;
        self.store(&state)
    }

    pub fn current_session(&self) -> io::Result<SessionState> {
        let mut session = self.ensure_session()?.0;
        // Prune-on-read: callers (`session_stats`, `catalog_sources` envelope)
        // never see stale entries past the TTL even if no fresh warning was
        // recorded since.
        session.prune_stale_warnings();
        Ok(session)
    }

    pub fn end_session(&self) -> io::Result<SessionState> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        state.current_session.prune_stale_warnings();
        if state.current_session.ended_at.is_none() {
            state.current_session.ended_at = Some(chrono::Utc::now().to_rfc3339());
            self.store(&state)?;
        }
        Ok(state.current_session)
    }

    pub fn record_fallback_to_native_tool(&self) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        state.current_session.blocked_grep_bash_attempts = state
            .current_session
            .blocked_grep_bash_attempts
            .saturating_add(1);
        self.store(&state)
    }

    /// Record a single `use <prefix>::…` rewrite observed by the PostToolUse
    /// hook. Returns the running count for that prefix after the increment.
    pub fn record_use_prefix_edit(&self, prefix: &str) -> io::Result<u64> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let entry = state
            .current_session
            .use_prefix_edits
            .entry(prefix.to_string())
            .or_default();
        *entry = entry.saturating_add(1);
        let count = *entry;
        self.store(&state)?;
        Ok(count)
    }

    /// Record file paths whose content was just returned by `read_content`.
    /// Paths are added to `dowser_read_paths` for later double-read
    /// detection by the PostToolUse Edit/Write hook.
    pub fn record_dowser_read_paths(&self, paths: &[String]) -> io::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        for p in paths {
            state.current_session.dowser_read_paths.insert(p.clone());
        }
        self.store(&state)
    }

    /// Record full-file reads and return the post-increment count for each
    /// path. Callers use the count to attach a `SUGGEST_OUTLINE` warning on
    /// the 2nd read and a harder `suggested_call` on the 3rd. Paths are
    /// bumped saturatingly so a single path can't overflow the `u8`.
    pub fn record_full_file_reads(&self, paths: &[String]) -> io::Result<Vec<(String, u8)>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let entry = state
                .current_session
                .full_file_read_counts
                .entry(p.clone())
                .or_insert(0);
            *entry = entry.saturating_add(1);
            out.push((p.clone(), *entry));
        }
        self.store(&state)?;
        Ok(out)
    }

    /// Hook-side probe: returns `true` if the edited `edit_path` had been
    /// returned by `read_content` earlier in this session, and increments
    /// the double-read counter. Because the hook sees absolute paths but
    /// the ledger stores repo-relative paths, we match on suffix.
    /// The content bytes are considered superseded because Claude Code's
    /// pre-Edit native Read re-reads the whole file regardless.
    pub fn record_edit_and_check_double_read(&self, edit_path: &str) -> io::Result<bool> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let was_read = state
            .current_session
            .dowser_read_paths
            .iter()
            .any(|rel| edit_path == rel || edit_path.ends_with(&format!("/{rel}")));
        if was_read {
            state.current_session.edits_after_dowser_read = state
                .current_session
                .edits_after_dowser_read
                .saturating_add(1);
            self.store(&state)?;
        }
        Ok(was_read)
    }

    pub fn record_stale_warning(&self, repo: &str, files_in_flight: &[String]) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        // Drop stale entries before appending — keeps the ledger from
        // accumulating an hours-old wall of warnings.
        state.current_session.prune_stale_warnings();
        let record = StaleWarningRecord {
            repo: repo.to_string(),
            files_in_flight: files_in_flight.to_vec(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        // Dedup ignores `recorded_at` so a re-record of the same in-flight
        // set doesn't bloat the ledger; the pre-TTL prune above handles
        // cleanup of *truly* stale entries.
        let already_present =
            state
                .current_session
                .stale_warnings_in_flight
                .iter()
                .any(|existing| {
                    existing.repo == record.repo
                        && existing.files_in_flight == record.files_in_flight
                });
        if already_present {
            return self.store(&state);
        }
        state.current_session.stale_warnings_in_flight.push(record);
        self.store(&state)
    }

    pub fn set_quality_session_start(&self, marker: QualitySnapshotMarker) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        if state.current_session.quality_session_start.is_none() {
            state.current_session.quality_session_start = Some(marker);
            self.store(&state)?;
        }
        Ok(())
    }

    pub fn set_quality_session_end(&self, marker: QualitySnapshotMarker) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        state.current_session.quality_session_end = Some(marker);
        self.store(&state)
    }

    fn load(&self) -> io::Result<SavingsState> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(SavingsState::default()),
            Err(err) => Err(err),
        }
    }

    fn store(&self, state: &SavingsState) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
        fs::write(&tmp_path, bytes)?;
        fs::rename(tmp_path, &self.path)
    }
}

fn ensure_active_session(state: &mut SavingsState) -> bool {
    let needs_new = state.current_session.id.is_empty()
        || state.current_session.started_at.is_empty()
        || state.current_session.ended_at.is_some();
    if needs_new {
        state.current_session = SessionState {
            id: format!("sess_{}", Uuid::new_v4()),
            started_at: chrono::Utc::now().to_rfc3339(),
            ended_at: None,
            tokens_saved_total: 0,
            per_tool: FxHashMap::default(),
            blocked_grep_bash_attempts: 0,
            impact_usage: FxHashMap::default(),
            stale_warnings_in_flight: Vec::new(),
            primary_source_id: None,
            quality_session_start: None,
            quality_session_end: None,
            use_prefix_edits: FxHashMap::default(),
            dowser_read_paths: std::collections::BTreeSet::new(),
            edits_after_dowser_read: 0,
            full_file_read_counts: FxHashMap::default(),
        };
    }
    needs_new
}
