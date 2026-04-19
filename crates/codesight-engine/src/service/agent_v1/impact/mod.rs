//! Impact analyses — dispatches `evaluate_impact` to per-analysis submodules.

mod blast_radius;
mod changed_units;
mod diff;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_impact(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateImpactArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();

        match a.as_str() {
            "diff" => self.evaluate_impact_diff(&args),
            "blast_radius" => self.evaluate_impact_blast_radius(&args),
            "changed_units" => self.evaluate_impact_changed_units(&args),
            "change_velocity" => self.evaluate_impact_change_velocity(&args),
            "impact_preview" => self.evaluate_impact_impact_preview(&args),
            "path_rename" => self.evaluate_impact_path_rename(&args),
            "rename_safety" => self.evaluate_impact_rename_safety(&args),
            _ => {
                let timer = MetaBuilder::new(0);
                let warnings = vec![json!({
                    "code": "NOT_IMPLEMENTED",
                    "message": format!("impact analysis '{a}' is not implemented."),
                })];
                self.success_response_with_warnings(
                    &timer,
                    json!({ "analysis": a, "findings": [] }),
                    warnings,
                )
            }
        }
    }
}
