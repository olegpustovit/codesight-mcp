//! `evaluate_quality analysis=test_gaps` — rank untested entry points.

use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn quality_test_gaps(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let top_n = args.top_n.unwrap_or(25);
        let min_complexity = args.min_complexity.unwrap_or(3);
        let data = codesight_analysis::test_gaps::test_gap_payload(&store, top_n, min_complexity)?;
        self.success_response(&timer, data)
    }
}
