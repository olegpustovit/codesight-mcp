//! `evaluate_quality analysis=hotspots` — top-N complexity rank.

use serde_json::{Value, json};

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

        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);
        let data = quality::hotspots_payload(&store, min_complexity, top_n, include_conventional)?;

        // ROADMAP P2.8: point the agent at concrete follow-up calls for the
        // top hotspots so they don't have to invent the next step.
        let top = data.pointer("/hotspots").and_then(Value::as_array);
        let actionable_next = top
            .map(|rows| {
                rows.iter()
                    .take(3)
                    .filter_map(|row| {
                        let name = row.get("name").and_then(Value::as_str)?;
                        let file = row.get("file").and_then(Value::as_str)?;
                        Some(json!({
                            "tool": "evaluate_impact",
                            "args": {
                                "source_id": args.source_id,
                                "analysis": "rename_safety",
                                "refs": [{ "kind": "symbol", "path": file, "symbol_name": name, "new_name": name }],
                            },
                            "reason": format!("Check rename safety on hotspot `{name}` ({file}) before splitting it."),
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [("actionable_next".to_string(), json!(actionable_next))],
        )
    }
}
