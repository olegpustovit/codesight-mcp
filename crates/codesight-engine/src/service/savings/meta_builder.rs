//! Response envelope metadata (`Meta`, `MetaBuilder`) and the retrieval
//! class taxonomy used to interpret `tokens_saved`.

use std::{collections::HashMap, time::Instant};

use serde::{Deserialize, Serialize};

use crate::config::{
    BUILD_SHA, BUILD_TIME, BUILD_VERSION, BYTES_PER_TOKEN, RETRIEVAL_ENGINE, RETRIEVAL_VERSION,
};

use super::ledger::SavingsLedger;
use super::pricing::{cost_for_tokens, round_cost_map};

/// Retrieval baseline class for `tokens_saved` — FEEDBACK.md §3b. A zero
/// `tokens_saved` on `whole_file_read` is expected (the caller asked for the
/// full file); a zero `tokens_saved` on `search` is a red flag indicating
/// the query wasn't narrow enough. Marking the class lets agents branch on
/// intent instead of treating every 0 as a misuse signal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalClass {
    /// Outlines, trees, class hierarchies — no source text returned.
    Structural,
    /// Specific symbol/section, or bounded `start_line..=end_line` slice.
    TargetedRange,
    /// Whole-file read via `read_content kind=file` with no line bounds.
    WholeFileRead,
    /// Pure metadata (`catalog_sources`, `manage_indexes`, …).
    MetadataOnly,
    /// BM25 / regex / fuzzy / dense search over an index.
    Search,
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    /// Omitted when the tool has no file-read baseline to compare against
    /// (e.g. `catalog_sources`, most `manage_indexes` actions). Agents should
    /// treat absence as "savings undefined", not "zero savings".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_saved: Option<u64>,
    /// FEEDBACK.md §3b: the baseline behind `tokens_saved`. Omitted when the
    /// caller didn't tag its `MetaBuilder`. See [`RetrievalClass`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_class: Option<RetrievalClass>,
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

#[derive(Debug)]
pub struct MetaBuilder {
    raw_bytes: u64,
    started_at: Instant,
    retrieval_class: Option<RetrievalClass>,
}

impl MetaBuilder {
    pub fn new(raw_bytes: u64) -> Self {
        Self {
            raw_bytes,
            started_at: Instant::now(),
            retrieval_class: None,
        }
    }

    /// Tag this response with its retrieval class (FEEDBACK.md §3b).
    #[must_use]
    pub fn with_class(mut self, class: RetrievalClass) -> Self {
        self.retrieval_class = Some(class);
        self
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
            retrieval_class: self.retrieval_class,
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
