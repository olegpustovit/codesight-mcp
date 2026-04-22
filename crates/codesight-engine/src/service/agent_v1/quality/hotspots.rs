//! `evaluate_quality analysis=hotspots` — top-N complexity rank.

use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::quality;

impl CodeService {
    pub(super) fn quality_hotspots(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let min_complexity = args.min_complexity.unwrap_or(0);
        let top_n = args.top_n.unwrap_or(50);
        let include_conventional = args.include_conventional.unwrap_or(false);

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let data =
            quality::hotspots_payload(&store, min_complexity, top_n, include_conventional)?;

        self.success_response(&timer, data)
    }
}
