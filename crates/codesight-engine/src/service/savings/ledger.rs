//! JSON-file-backed savings ledger. Tracks cumulative token savings,
//! per-session tool counters, and related markers. Writes are atomic
//! (tmp file + rename) and reads tolerate missing files.

use std::{collections::HashMap, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::pricing::{cost_for_tokens, round_cost, round_cost_map};
use super::session::{
    QualitySnapshotMarker, ReadContentStats, SessionState, StaleWarningRecord,
};

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavingsState {
    total_tokens_saved: u64,
    #[serde(default)]
    total_cost_avoided: HashMap<String, f64>,
    #[serde(default)]
    current_session: SessionState,
}

#[derive(Debug, Clone)]
pub struct SavingsLedger {
    path: PathBuf,
}

impl SavingsLedger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn current_total(&self) -> io::Result<u64> {
        Ok(self.load()?.total_tokens_saved)
    }

    pub fn record(&self, delta: u64) -> io::Result<(u64, HashMap<String, f64>)> {
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

    pub fn current_state(&self) -> io::Result<(u64, HashMap<String, f64>)> {
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

    pub fn record_tool_call(
        &self,
        tool: &str,
        tokens_saved: u64,
        timing_ms: f64,
        ok: bool,
        source_id: Option<&str>,
        impact_analysis: Option<&str>,
    ) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let session = &mut state.current_session;
        if session.primary_source_id.is_none()
            && let Some(source_id) = source_id.filter(|value| !value.is_empty())
        {
            session.primary_source_id = Some(source_id.to_string());
        }
        session.tokens_saved_total = session.tokens_saved_total.saturating_add(tokens_saved);
        if tool == "evaluate_impact"
            && let Some(analysis) = impact_analysis.filter(|value| !value.is_empty())
        {
            *session
                .evaluate_impact_usage
                .entry(analysis.to_string())
                .or_default() += 1;
        }
        let entry = session.per_tool.entry(tool.to_string()).or_default();
        entry.calls = entry.calls.saturating_add(1);
        entry.tokens_saved = entry.tokens_saved.saturating_add(tokens_saved);
        entry.timing_ms_sum += timing_ms;
        if !ok {
            entry.error_count = entry.error_count.saturating_add(1);
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
        Ok(self.ensure_session()?.0)
    }

    pub fn end_session(&self) -> io::Result<SessionState> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        if state.current_session.ended_at.is_none() {
            state.current_session.ended_at = Some(chrono::Utc::now().to_rfc3339());
            self.store(&state)?;
        }
        Ok(state.current_session)
    }

    pub fn record_fallback_to_native_tool(&self) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        state.current_session.fallbacks_to_grep_or_bash = state
            .current_session
            .fallbacks_to_grep_or_bash
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
    /// Paths are added to `codesight_read_paths` for later double-read
    /// detection by the PostToolUse Edit/Write hook.
    pub fn record_codesight_read_paths(&self, paths: &[String]) -> io::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        for p in paths {
            state.current_session.codesight_read_paths.insert(p.clone());
        }
        self.store(&state)
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
            .codesight_read_paths
            .iter()
            .any(|rel| edit_path == rel || edit_path.ends_with(&format!("/{rel}")));
        if was_read {
            state.current_session.edits_after_codesight_read = state
                .current_session
                .edits_after_codesight_read
                .saturating_add(1);
            self.store(&state)?;
        }
        Ok(was_read)
    }

    pub fn record_stale_warning(&self, repo: &str, files_in_flight: &[String]) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        let record = StaleWarningRecord {
            repo: repo.to_string(),
            files_in_flight: files_in_flight.to_vec(),
        };
        if state
            .current_session
            .stale_warnings_fired_with_in_flight_paths
            .contains(&record)
        {
            return Ok(());
        }
        state
            .current_session
            .stale_warnings_fired_with_in_flight_paths
            .push(record);
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
            per_tool: HashMap::new(),
            fallbacks_to_grep_or_bash: 0,
            evaluate_impact_usage: HashMap::new(),
            stale_warnings_fired_with_in_flight_paths: Vec::new(),
            primary_source_id: None,
            quality_session_start: None,
            quality_session_end: None,
            use_prefix_edits: HashMap::new(),
            codesight_read_paths: std::collections::BTreeSet::new(),
            edits_after_codesight_read: 0,
        };
    }
    needs_new
}
