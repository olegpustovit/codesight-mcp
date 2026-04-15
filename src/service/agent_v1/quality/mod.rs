pub mod doc_coverage;
mod repo_health;

use serde_json::{Value, json};

use codesight_analysis::{dead_code, quality, quality_signal};
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match a.as_str() {
            "complexity" => self.quality_complexity(&args, &timer),
            "repo_health" => self.quality_repo_health(&args, &timer),
            "dead_code" => self.quality_dead_code(&args, &timer),
            "dead_code_v2" => self.quality_dead_code_v2(&args, &timer),
            "quality_signal" => self.quality_signal_handler(&args, &timer),
            "regression_check" => self.quality_regression_check(&args, &timer),
            tail => self.evaluate_quality_tail(&args, tail, &timer),
        }
    }

    fn quality_complexity(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let Some(r) = args.refs.as_ref().and_then(|x| x.first()) else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "complexity requires refs[0].".to_string(),
                    detail: json!({}),
                },
            );
        };
        let Some(id) = r
            .stable_id
            .clone()
            .or(r.id.clone())
            .filter(|_| r.kind.eq_ignore_ascii_case("symbol"))
        else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "complexity requires a symbol ref.".to_string(),
                    detail: json!({}),
                },
            );
        };
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let data = match quality::complexity_payload(&store, &id)? {
            Some(data) => data,
            None => {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{id}' not found."),
                        detail: json!({ "id": id }),
                    },
                );
            }
        };
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        self.success_response(&timer, data)
    }

    fn quality_dead_code(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let granularity = args.granularity.as_deref().unwrap_or("file");
        let min_confidence = args.min_confidence.unwrap_or(0.5);
        let include_tests = args.include_tests.unwrap_or(true);
        let entry_patterns = args.entry_point_patterns.clone().unwrap_or_default();
        let limit = args.top_n.unwrap_or(25);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let mut data = dead_code::dead_code_payload(
            &store,
            granularity,
            min_confidence,
            include_tests,
            &entry_patterns,
            Some(limit),
        )?;
        if let Some(budget) = args.max_tokens {
            dead_code::pack_dead_code_to_budget(&mut data, budget);
        }
        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [
                ("methodology".to_string(), json!("multi_signal")),
                ("confidence_level".to_string(), json!("medium")),
            ],
        )
    }

    fn quality_dead_code_v2(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let min_confidence = args.min_confidence.unwrap_or(0.5);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let data = dead_code::dead_code_v2_payload(&store, min_confidence)?;
        self.success_response(&timer, data)
    }

    fn quality_signal_handler(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);

        let gov = quality_signal::compute_governance_signal(&store)?;
        let suggestion =
            quality_signal::suggestion_for_bottleneck(gov.bottleneck, &gov.structural.coverage);
        let mut data = quality_signal::governance_signal_json(&gov, &suggestion);
        // quality_signal response adds analysis label and coverage block.
        let obj = data.as_object_mut().unwrap();
        obj.insert("analysis".into(), json!("quality_signal"));
        obj.insert(
            "coverage".into(),
            json!({
                "graph_files": gov.structural.coverage.graph_files,
                "graph_edges": gov.structural.coverage.graph_edges,
                "functions_with_source": gov.structural.coverage.functions_with_source,
                "dimensions_available": gov.structural.coverage.dimensions_available,
            }),
        );
        self.success_response(&timer, data)
    }

    fn quality_regression_check(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let snapshot_name = args.snapshot_name.as_deref().unwrap_or("session");
        let baseline = match store.load_snapshot(&args.source_id, snapshot_name)? {
            Some(b) => b,
            None => {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NO_BASELINE",
                        message: format!(
                            "No snapshot '{snapshot_name}' found. Run manage_indexes action=snapshot_quality first."
                        ),
                        detail: json!({ "snapshot_name": snapshot_name }),
                    },
                );
            }
        };

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);

        let gov = quality_signal::compute_governance_signal(&store)?;

        // Compute deltas
        let d_score = gov.score as i64 - baseline.score as i64;
        let d_structural = gov.structural_score as i64 - baseline.structural_score as i64;
        let d_agent = gov.agent_readiness_score as i64 - baseline.agent_readiness_score as i64;

        let s = &gov.structural.root_causes;
        let a = &gov.agent.dimensions;
        let b = &baseline;
        let dim_deltas: [(&str, Option<f64>, Option<f64>); 9] = [
            ("modularity", s.modularity, b.modularity),
            ("acyclicity", Some(s.acyclicity), b.acyclicity),
            ("depth", s.depth, b.depth),
            ("equality", Some(s.equality), b.equality),
            ("redundancy", Some(s.redundancy), b.redundancy),
            ("navigability", Some(a.navigability), b.navigability),
            (
                "comprehensibility",
                Some(a.comprehensibility),
                b.comprehensibility,
            ),
            ("verifiability", Some(a.verifiability), b.verifiability),
            ("change_safety", Some(a.change_safety), b.change_safety),
        ];

        let mut degraded_dims = Vec::new();
        let mut improved_dims = Vec::new();
        let mut findings = Vec::new();
        let mut delta_obj = json!({
            "score": d_score,
            "structural_score": d_structural,
            "agent_readiness_score": d_agent,
        });

        for (name, current, baseline_val) in &dim_deltas {
            if let (Some(c), Some(b)) = (current, baseline_val) {
                let delta = c - b;
                delta_obj[name] = json!(delta);
                if delta < -0.05 {
                    degraded_dims.push(*name);
                    let severity = if delta < -0.15 { "error" } else { "warning" };
                    findings.push(json!({
                        "finding_id": format!("regression:{name}"),
                        "severity": severity,
                        "summary": format!("{name} dropped {b:.2} → {c:.2} (delta {delta:+.2})"),
                        "evidence": { "before": b, "after": c, "delta": delta },
                    }));
                } else if delta > 0.05 {
                    improved_dims.push(*name);
                }
            }
        }

        let degraded = d_score < -100 || d_structural < -100 || !degraded_dims.is_empty();

        let suggestion = if let Some(worst) = degraded_dims.first() {
            quality_signal::suggestion_for_bottleneck(worst, &gov.structural.coverage)
        } else if degraded {
            "Overall score decreased; investigate recent changes".to_string()
        } else {
            "No significant regression detected".to_string()
        };

        self.success_response(
            &timer,
            json!({
                "analysis": "regression_check",
                "baseline": {
                    "source_id": baseline.source_id,
                    "snapshot_name": baseline.name,
                    "created_at": baseline.created_at,
                    "score": baseline.score,
                    "structural_score": baseline.structural_score,
                    "agent_readiness_score": baseline.agent_readiness_score,
                    "agent_dimensions": {
                        "navigability": baseline.navigability,
                        "comprehensibility": baseline.comprehensibility,
                        "verifiability": baseline.verifiability,
                        "change_safety": baseline.change_safety,
                    },
                },
                "current": {
                    "score": gov.score,
                    "structural_score": gov.structural_score,
                    "agent_readiness_score": gov.agent_readiness_score,
                    "agent_dimensions": {
                        "navigability": gov.agent.dimensions.navigability,
                        "comprehensibility": gov.agent.dimensions.comprehensibility,
                        "verifiability": gov.agent.dimensions.verifiability,
                        "change_safety": gov.agent.dimensions.change_safety,
                    },
                },
                "delta": delta_obj,
                "degraded": degraded,
                "degraded_dimensions": degraded_dims,
                "improved_dimensions": improved_dims,
                "suggestion": suggestion,
                "findings": findings,
            }),
        )
    }
}
