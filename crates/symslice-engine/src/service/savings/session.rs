//! Per-session counters persisted in the savings ledger.

use rustc_hash::FxHashMap;

use serde::{Deserialize, Serialize};

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
    pub per_tool: FxHashMap<String, ToolSessionStats>,
    #[serde(default)]
    pub blocked_grep_bash_attempts: u64,
    #[serde(default)]
    pub impact_usage: FxHashMap<String, u64>,
    #[serde(default)]
    pub stale_warnings_in_flight: Vec<StaleWarningRecord>,
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
    pub use_prefix_edits: FxHashMap<String, u64>,
    /// Paths whose content was returned via `read_content` during this
    /// session. Populated on every file-returning `read_content` call.
    /// Used to detect the Claude-Code double-read penalty: an Edit on one
    /// of these paths re-reads the whole file via the harness's native Read
    /// contract, making the symslice read's savings illusory.
    #[serde(default)]
    pub symslice_read_paths: std::collections::BTreeSet<String>,
    /// Count of Edit/Write events whose `file_path` was previously returned
    /// by `read_content` in this session. Each one represents a file whose
    /// symslice bytes were superseded by the harness's pre-Edit read.
    #[serde(default)]
    pub edits_after_symslice_read: u64,
    /// Per-path count of full-file `read_content` calls during this session.
    /// Populated by [`crate::service::CodeService::dispatch`] after every
    /// successful `read_content` call that returned a whole file. Drives
    /// the `SUGGEST_OUTLINE` nudge — the 2nd read adds a warning, the 3rd+
    /// promotes to a suggested_call. Narrow reads (symbol/section/range)
    /// don't count.
    #[serde(default)]
    pub full_file_read_counts: FxHashMap<String, u8>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StaleWarningRecord {
    pub repo: String,
    #[serde(default)]
    pub files_in_flight: Vec<String>,
    /// RFC3339 timestamp captured at the time of recording. Used by
    /// [`SessionState::prune_stale_warnings`] to drop entries past
    /// [`STALE_WARNING_TTL_SECS`] so a long-running session doesn't surface
    /// in-flight files from days ago. Empty when deserialized from a
    /// pre-TTL ledger; treated as "definitely stale" on the next read.
    #[serde(default)]
    pub recorded_at: String,
}

/// Lifetime, in seconds, after which a `stale_warnings_in_flight` entry is
/// dropped from the session ledger. Five minutes is long enough to cover an
/// in-flight reindex of any reasonable repo, short enough that a session
/// alive for hours never accumulates a wall of historical noise.
pub const STALE_WARNING_TTL_SECS: i64 = 300;

impl SessionState {
    /// Drop `stale_warnings_in_flight` entries whose `recorded_at` is older
    /// than [`STALE_WARNING_TTL_SECS`]. Call before serializing the session
    /// in `session_stats` / `session_end` so callers never see ghost
    /// warnings from prior reindex cycles.
    pub fn prune_stale_warnings(&mut self) {
        let cutoff = chrono::Utc::now()
            - chrono::Duration::try_seconds(STALE_WARNING_TTL_SECS)
                .unwrap_or_else(chrono::Duration::zero);
        self.stale_warnings_in_flight.retain(|record| {
            if record.recorded_at.is_empty() {
                // Pre-TTL records have no timestamp; treat as expired so a
                // legacy ledger doesn't keep its history forever.
                return false;
            }
            chrono::DateTime::parse_from_rfc3339(&record.recorded_at)
                .map(|t| t > cutoff)
                .unwrap_or(false)
        });
    }
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
