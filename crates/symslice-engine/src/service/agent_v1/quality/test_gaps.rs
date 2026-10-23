//! `health analysis=test_gaps` — untested high-risk symbols ranked by
//! `cyclomatic × (1 + fan_in) × (1 + ln(1 + churn))`.
//!
//! Tier 6.1: surfaces the existing
//! [`symslice_analysis::test_gaps::test_gap_payload`] computation as a real
//! analysis on the public `health` tool. The recommendations system in
//! `symslice_analysis::quality_signal::recommendations` points here when
//! `test_oracle_quality` is the bottleneck dimension.

use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use symslice_analysis::test_gaps;

impl CodeService {
    pub(super) fn quality_test_gaps(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let top_n = args.top_n.unwrap_or(50);
        let min_complexity = args.min_complexity.unwrap_or(0);

        // Heuristic risk-rank (cyclomatic × fan_in × churn) — partial
        // confidence: a low-risk-but-fragile function may slip the threshold
        // and a low-complexity-but-critical one may rank lower than ideal.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);
        let data = test_gaps::test_gap_payload(&store, top_n, min_complexity)?;
        self.success_response_with_warnings(&timer, data, Vec::new())
    }
}
