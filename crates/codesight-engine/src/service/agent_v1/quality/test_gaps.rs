//! `evaluate_quality analysis=test_gaps` — rank untested entry points.

use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

impl CodeService {
    pub(super) fn quality_test_gaps(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        // ROADMAP P1.1: quality analysis — null baseline.
        // Test-gap ranking is heuristic (entry-point detection, naming).
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Heuristic);
        let top_n = args.top_n.unwrap_or(25);
        let min_complexity = args.min_complexity.unwrap_or(3);
        let data = codesight_analysis::test_gaps::test_gap_payload(&store, top_n, min_complexity)?;
        self.success_response(&timer, data)
    }
}
