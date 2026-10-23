//! Shared on-disk cache format for `evaluate_quality analysis=quality_signal scope=…`
//! agent-readiness probes. The service writes entries; the hooks read them to
//! decide whether to surface refactor hints during `Edit`/`Write`.

use rustc_hash::FxHashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "_agent_readiness_cache.json";
pub const TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub entries: FxHashMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub score: f64,
    pub weakest_dimension: String,
    pub threshold: f64,
    pub computed_at: String,
}
