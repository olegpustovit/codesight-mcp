//! `evaluate_quality analysis=hotspots` — top-N complexity rank.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use symslice_analysis::quality;

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
        // Hotspots aggregate complexity + churn + size — partial.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);
        let data = quality::hotspots_payload(&store, min_complexity, top_n, include_conventional)?;

        // Tier 6.1: point the agent at a real follow-up tool. Previously this
        // emitted `evaluate_quality analysis=refactor_targets`, neither of
        // which exist on the public surface. Use `impact analysis=blast_radius`
        // — that tells the caller how disruptive a refactor of the hotspot
        // function will be before they touch it.
        let top = data.pointer("/hotspots").and_then(Value::as_array);
        let actionable_next = top
            .map(|rows| {
                rows.iter()
                    .take(3)
                    .filter_map(|row| {
                        let name = row.get("name").and_then(Value::as_str)?;
                        let file = row.get("file").and_then(Value::as_str)?;
                        let kind = row.get("kind").and_then(Value::as_str).unwrap_or("function");
                        Some(json!({
                            "tool": "impact",
                            "args": {
                                "analysis": "blast_radius",
                                "ref": format!("{file}::{name}#{kind}"),
                            },
                            "reason": format!("Blast radius for `{name}` ({file}) — gauge cost before refactoring this hotspot."),
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
