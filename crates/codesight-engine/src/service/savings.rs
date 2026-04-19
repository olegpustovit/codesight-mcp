use std::{collections::HashMap, fs, io, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{
    BUILD_SHA, BUILD_TIME, BUILD_VERSION, BYTES_PER_TOKEN, RETRIEVAL_ENGINE, RETRIEVAL_VERSION,
};

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    /// Omitted when the tool has no file-read baseline to compare against
    /// (e.g. `catalog_sources`, most `manage_indexes` actions). Agents should
    /// treat absence as "savings undefined", not "zero savings".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_saved: Option<u64>,
    pub total_tokens_saved: u64,
    pub response_tokens: u64,
    pub naive_tokens: u64,
    pub cost_avoided: HashMap<String, f64>,
    pub total_cost_avoided: HashMap<String, f64>,
    pub retrieval_engine: String,
    pub retrieval_version: String,
    /// Semver of the running codesight-mcp binary (`CARGO_PKG_VERSION`).
    pub build_version: String,
    /// Short git SHA of the commit this binary was built from, or
    /// `"unknown"` when the build ran outside a git checkout.
    pub build_sha: String,
    /// Build timestamp in Unix epoch seconds (stringified). Lets clients
    /// spot a long-running MCP server that predates a fix landing on disk.
    pub build_time: String,
    pub timing_ms: f64,
    pub tokens_saved_per_ms: f64,
}

// Per-million-token input pricing (USD)
const MODEL_PRICING: &[(&str, f64)] = &[
    ("claude_opus", 15.0),
    ("claude_sonnet", 3.0),
    ("gpt4o", 2.50),
];

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ReadContentStats {
    pub symbol_reads: u64,
    pub section_reads: u64,
    pub file_range_reads: u64,
    pub file_full_reads: u64,
    pub warning_count: u64,
    pub bytes_returned: u64,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ToolSessionStats {
    pub calls: u64,
    pub tokens_saved: u64,
    pub timing_ms_sum: f64,
    pub error_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_content_detail: Option<ReadContentStats>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct SessionState {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub tokens_saved_total: u64,
    #[serde(default)]
    pub per_tool: HashMap<String, ToolSessionStats>,
    #[serde(default)]
    pub fallbacks_to_grep_or_bash: u64,
    #[serde(default)]
    pub evaluate_impact_usage: HashMap<String, u64>,
    #[serde(default)]
    pub stale_warnings_fired_with_in_flight_paths: Vec<StaleWarningRecord>,
    #[serde(default)]
    pub primary_source_id: Option<String>,
    #[serde(default)]
    pub quality_session_start: Option<QualitySnapshotMarker>,
    #[serde(default)]
    pub quality_session_end: Option<QualitySnapshotMarker>,
    /// Count of Edit/Write events that rewrote `use <prefix>::…` imports,
    /// keyed by the shared source prefix. Populated by the PostToolUse hook.
    /// Used by `session_stats` to flag refactor-shaped edits that never called
    /// `evaluate_impact analysis=blast_radius`.
    #[serde(default)]
    pub use_prefix_edits: HashMap<String, u64>,
    /// Paths whose content was returned via `read_content` during this
    /// session. Populated on every file-returning `read_content` call.
    /// Used to detect the Claude-Code double-read penalty: an Edit on one
    /// of these paths re-reads the whole file via the harness's native Read
    /// contract, making the codesight read's savings illusory.
    #[serde(default)]
    pub codesight_read_paths: std::collections::BTreeSet<String>,
    /// Count of Edit/Write events whose `file_path` was previously returned
    /// by `read_content` in this session. Each one represents a file whose
    /// codesight bytes were superseded by the harness's pre-Edit read.
    #[serde(default)]
    pub edits_after_codesight_read: u64,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StaleWarningRecord {
    pub repo: String,
    #[serde(default)]
    pub files_in_flight: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct QualityHotspotMarker {
    pub file: String,
    pub cyclomatic: u64,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
pub struct QualitySnapshotMarker {
    pub source_id: String,
    pub snapshot_name: String,
    #[serde(default)]
    pub avg_complexity: Option<f64>,
    #[serde(default)]
    pub hotspots: Vec<QualityHotspotMarker>,
}

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

    pub fn set_quality_session_start(
        &self,
        marker: QualitySnapshotMarker,
    ) -> io::Result<()> {
        let mut state = self.load()?;
        ensure_active_session(&mut state);
        if state.current_session.quality_session_start.is_none() {
            state.current_session.quality_session_start = Some(marker);
            self.store(&state)?;
        }
        Ok(())
    }

    pub fn set_quality_session_end(
        &self,
        marker: QualitySnapshotMarker,
    ) -> io::Result<()> {
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

fn cost_for_tokens(tokens: u64) -> HashMap<String, f64> {
    MODEL_PRICING
        .iter()
        .map(|&(model, price)| {
            let cost = (tokens as f64 / 1_000_000.0) * price;
            (model.to_string(), round_cost(cost))
        })
        .collect()
}

fn round_cost(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn round_cost_map(mut values: HashMap<String, f64>) -> HashMap<String, f64> {
    for value in values.values_mut() {
        *value = round_cost(*value);
    }
    values
}

#[derive(Debug)]
pub struct MetaBuilder {
    raw_bytes: u64,
    started_at: Instant,
}

impl MetaBuilder {
    pub fn new(raw_bytes: u64) -> Self {
        Self {
            raw_bytes,
            started_at: Instant::now(),
        }
    }

    pub fn success<T: serde::Serialize>(
        &self,
        body: &T,
        ledger: &SavingsLedger,
    ) -> anyhow::Result<Meta> {
        let response_bytes = serde_json::to_vec(body)?.len() as u64;
        let response_tokens = response_bytes / BYTES_PER_TOKEN;
        let naive_tokens = self.raw_bytes / BYTES_PER_TOKEN;
        let tokens_saved = naive_tokens.saturating_sub(response_tokens);
        let (total_tokens_saved, total_cost_avoided) = ledger.record(tokens_saved)?;
        // `raw_bytes == 0` means the tool has no file-read baseline to
        // compare against — surfacing `tokens_saved: 0` in that case is
        // misleading. Omit the key entirely for those tools.
        let tokens_saved_opt = if self.raw_bytes == 0 {
            None
        } else {
            Some(tokens_saved)
        };
        Ok(self.finish(
            tokens_saved_opt,
            total_tokens_saved,
            response_tokens,
            naive_tokens,
            cost_for_tokens(tokens_saved),
            total_cost_avoided,
        ))
    }

    pub fn error(&self, ledger: &SavingsLedger) -> anyhow::Result<Meta> {
        let (total_tokens_saved, total_cost_avoided) = ledger.current_state()?;
        Ok(self.finish(
            None,
            total_tokens_saved,
            0,
            self.raw_bytes / BYTES_PER_TOKEN,
            HashMap::new(),
            total_cost_avoided,
        ))
    }

    fn finish(
        &self,
        tokens_saved: Option<u64>,
        total_tokens_saved: u64,
        response_tokens: u64,
        naive_tokens: u64,
        cost_avoided: HashMap<String, f64>,
        total_cost_avoided: HashMap<String, f64>,
    ) -> Meta {
        let raw_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
        // Round timing_ms to 2 decimals at the serialization boundary.
        // Avoids `0.46299999999999996`-style noise that bloats every response
        // with zero information gain (PLAN2 §3.6).
        let timing_ms = (raw_ms * 100.0).round() / 100.0;
        let tokens_saved_per_ms_raw = tokens_saved.unwrap_or(0) as f64 / raw_ms.max(0.1);
        let tokens_saved_per_ms = (tokens_saved_per_ms_raw * 100.0).round() / 100.0;
        Meta {
            tokens_saved,
            total_tokens_saved,
            response_tokens,
            naive_tokens,
            cost_avoided: round_cost_map(cost_avoided),
            total_cost_avoided: round_cost_map(total_cost_avoided),
            retrieval_engine: RETRIEVAL_ENGINE.to_string(),
            retrieval_version: RETRIEVAL_VERSION.to_string(),
            build_version: BUILD_VERSION.to_string(),
            build_sha: BUILD_SHA.to_string(),
            build_time: BUILD_TIME.to_string(),
            timing_ms,
            tokens_saved_per_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::SavingsLedger;

    #[test]
    fn records_codesight_read_paths_and_detects_double_read() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        ledger
            .record_codesight_read_paths(&[
                "src/foo.rs".to_string(),
                "src/bar.rs".to_string(),
            ])
            .unwrap();
        // Absolute path from the hook side should match on suffix.
        let was_read = ledger
            .record_edit_and_check_double_read("/tmp/repo/src/foo.rs")
            .unwrap();
        assert!(was_read);
        // Unseen file is not a double-read.
        let not_read = ledger
            .record_edit_and_check_double_read("/tmp/repo/src/baz.rs")
            .unwrap();
        assert!(!not_read);
        let session = ledger.current_session().unwrap();
        assert_eq!(session.edits_after_codesight_read, 1);
    }

    #[test]
    fn savings_ledger_rounds_accumulated_costs() {
        let dir = tempdir().unwrap();
        let ledger = SavingsLedger::new(dir.path().join("_savings.json"));
        ledger.record(123_456).unwrap();
        ledger.record(123_456).unwrap();
        let (_tokens, costs) = ledger.current_state().unwrap();
        for value in costs.values() {
            let scaled = value * 10_000.0;
            assert!(
                (scaled - scaled.round()).abs() < 1e-9,
                "cost should be rounded to 4 decimals: {value}"
            );
        }
    }
}
