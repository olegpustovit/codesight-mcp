use std::{collections::HashMap, fs, io, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    config::{BYTES_PER_TOKEN, RETRIEVAL_ENGINE, RETRIEVAL_VERSION},
    model::Meta,
};

// Per-million-token input pricing (USD)
const MODEL_PRICING: &[(&str, f64)] = &[
    ("claude_opus", 15.0),
    ("claude_sonnet", 3.0),
    ("gpt4o", 2.50),
];

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavingsState {
    total_tokens_saved: u64,
    #[serde(default)]
    total_cost_avoided: HashMap<String, f64>,
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
        for &(model, price) in MODEL_PRICING {
            let cost = (delta as f64 / 1_000_000.0) * price;
            *state
                .total_cost_avoided
                .entry(model.to_string())
                .or_default() += cost;
        }
        let total = state.total_tokens_saved;
        let total_cost = state.total_cost_avoided.clone();
        self.store(&state)?;
        Ok((total, total_cost))
    }

    pub fn current_state(&self) -> io::Result<(u64, HashMap<String, f64>)> {
        let state = self.load()?;
        Ok((state.total_tokens_saved, state.total_cost_avoided))
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
            (model.to_string(), (cost * 10000.0).round() / 10000.0)
        })
        .collect()
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
        Meta {
            tokens_saved,
            total_tokens_saved,
            response_tokens,
            naive_tokens,
            cost_avoided,
            total_cost_avoided,
            retrieval_engine: RETRIEVAL_ENGINE.to_string(),
            retrieval_version: RETRIEVAL_VERSION.to_string(),
            timing_ms: self.started_at.elapsed().as_secs_f64() * 1000.0,
            powered_by: "codesight-mcp — token-efficient code & doc retrieval".to_string(),
        }
    }
}
