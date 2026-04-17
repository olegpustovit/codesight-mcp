use std::{collections::HashMap, fs, io, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::config::{
    BUILD_SHA, BUILD_TIME, BUILD_VERSION, BYTES_PER_TOKEN, RETRIEVAL_ENGINE, RETRIEVAL_VERSION,
};

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub tokens_saved: u64,
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
    pub powered_by: String,
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
    pub started_at: String,
    #[serde(default)]
    pub per_tool: HashMap<String, ToolSessionStats>,
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

    pub fn record_tool_call(
        &self,
        tool: &str,
        tokens_saved: u64,
        timing_ms: f64,
        ok: bool,
    ) -> io::Result<()> {
        let mut state = self.load()?;
        if state.current_session.started_at.is_empty() {
            state.current_session.started_at = chrono::Utc::now().to_rfc3339();
        }
        let entry = state
            .current_session
            .per_tool
            .entry(tool.to_string())
            .or_default();
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
        Ok(self.load()?.current_session)
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
        Ok(self.finish(
            tokens_saved,
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
            0,
            total_tokens_saved,
            0,
            self.raw_bytes / BYTES_PER_TOKEN,
            HashMap::new(),
            total_cost_avoided,
        ))
    }

    fn finish(
        &self,
        tokens_saved: u64,
        total_tokens_saved: u64,
        response_tokens: u64,
        naive_tokens: u64,
        cost_avoided: HashMap<String, f64>,
        total_cost_avoided: HashMap<String, f64>,
    ) -> Meta {
        let timing_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
        let tokens_saved_per_ms = tokens_saved as f64 / timing_ms.max(0.1);
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
            powered_by: "codesight-mcp — token-efficient code & doc retrieval".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::SavingsLedger;

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
