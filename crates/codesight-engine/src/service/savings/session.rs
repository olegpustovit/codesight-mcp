//! Per-session counters persisted in the savings ledger.

use std::collections::HashMap;

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
