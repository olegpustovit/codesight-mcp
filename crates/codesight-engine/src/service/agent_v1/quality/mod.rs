mod broken_links;
pub mod doc_coverage;
mod hotspots;
mod refactor_targets;
mod repo_health;
mod test_gaps;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::{
    agent_readiness::compute_agent_readiness_for_scope, dead_code, quality, quality_signal,
    thresholds::current_thresholds,
};

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);
        // ROADMAP P3.6: wall-clock budget. Wrap the inner dispatch so every
        // analysis handler plays by the same rules: when `max_wait_ms` is
        // set and elapsed time exceeds it, either error out
        // (`DEADLINE_EXCEEDED`) or return the finished payload stamped with
        // a `BUDGET_EXCEEDED` warning when `partial_ok=true`. Finer-grained
        // cancellation inside the heavier handlers is follow-up work — this
        // contract gives callers the bound they can act on now.
        let start = std::time::Instant::now();
        let inner = match a.as_str() {
            "complexity" => self.quality_complexity(&args, &timer),
            "repo_health" => self.quality_repo_health(&args, &timer),
            "dead_code" => self.quality_dead_code(&args, &timer),
            "dead_code_v2" => self.quality_dead_code_v2(&args, &timer),
            "quality_signal" => self.quality_signal_handler(&args, &timer),
            "regression_check" => self.quality_regression_check(&args, &timer),
            "hotspots" => self.quality_hotspots(&args, &timer),
            "refactor_targets" => self.quality_refactor_targets(&args, &timer),
            "broken_links" => self.quality_broken_links(&args, &timer),
            "doc_coverage" => self.quality_doc_coverage(&args, &timer),
            "test_gaps" => self.quality_test_gaps(&args, &timer),
            unknown => self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown quality analysis '{unknown}'."),
                    detail: json!({ "analysis": unknown }),
                },
            ),
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let mut response = inner?;
        if let Some(budget) = args.max_wait_ms
            && elapsed_ms > budget
        {
            if args.partial_ok.unwrap_or(false) {
                if let Some(body) = response.as_object_mut()
                    && let Some(Value::Array(warns)) =
                        body.entry("warnings".to_string())
                            .or_insert_with(|| Value::Array(Vec::new()))
                            .as_array_mut()
                            .map(|a| Value::Array(std::mem::take(a)))
                            .and_then(|v| {
                                body.insert("warnings".to_string(), v);
                                body.get_mut("warnings")
                            })
                {
                    warns.push(json!({
                        "code": "BUDGET_EXCEEDED",
                        "message": format!(
                            "analysis took {elapsed_ms} ms which exceeds max_wait_ms={budget}; \
                             returning partial result per partial_ok=true.",
                        ),
                        "detail": { "elapsed_ms": elapsed_ms, "budget_ms": budget },
                    }));
                }
            } else {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::Internal,
                        code: "DEADLINE_EXCEEDED",
                        message: format!(
                            "analysis took {elapsed_ms} ms which exceeds max_wait_ms={budget}; \
                             pass partial_ok=true to accept partial results.",
                        ),
                        detail: json!({ "elapsed_ms": elapsed_ms, "budget_ms": budget }),
                    },
                );
            }
        }
        if let Some(meta) = response.get_mut("_meta").and_then(Value::as_object_mut) {
            meta.insert("thresholds".to_string(), current_thresholds().as_json());
        }
        Ok(response)
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
        if !r.kind.eq_ignore_ascii_case("symbol") {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "complexity requires a symbol ref.".to_string(),
                    detail: json!({ "ref": r }),
                },
            );
        }
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        use crate::service::agent_v1::util::{
            ref_requires_strict_resolution, resolve_symbol_id_strict,
        };
        let is_strict = ref_requires_strict_resolution(
            r.stable_id.as_deref(),
            r.id.as_deref(),
            r.path.as_deref(),
            r.start_line,
            r.symbol_name.as_deref(),
        );
        let resolved = resolve_symbol_id_strict(
            &store,
            r.stable_id.as_deref(),
            r.id.as_deref(),
            r.path.as_deref(),
            r.start_line,
            r.symbol_name.as_deref(),
        )?;
        let (id, resolution) = match resolved {
            Some((id, strategy)) => (id, Some(strategy.as_str())),
            None if is_strict => {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: "complexity could not resolve refs[0].".to_string(),
                        detail: json!({ "unresolved": [r] }),
                    },
                );
            }
            None => {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "complexity ref provided no resolvable fields.".to_string(),
                        detail: json!({ "ref": r }),
                    },
                );
            }
        };
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
        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);
        // ROADMAP P0.2 #3: echo which ref shape resolved the target.
        let mut extras: Vec<(String, Value)> = Vec::new();
        if let Some(r) = resolution {
            extras.push(("resolution".to_string(), json!(r)));
        }
        self.success_response_with_meta_extra(&timer, data, Vec::new(), extras)
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
        let include_vendored = args.include_vendored.unwrap_or(false)
            || args
                .scope
                .as_deref()
                .map(codesight_language_support::scope_opts_in)
                .unwrap_or(false);
        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);
        let mut data = dead_code::dead_code_payload(
            &store,
            granularity,
            min_confidence,
            include_tests,
            &entry_patterns,
            Some(limit),
            include_vendored,
        )?;
        if let Some(budget) = args.max_tokens {
            dead_code::pack_dead_code_to_budget(&mut data, budget);
        }
        // ROADMAP P2.8: emit actionable_next hints pointing at the top-3
        // flagged files so the agent can confirm/reject via find_references
        // in one round-trip instead of guessing which hit to probe.
        let hints: Vec<Value> = data
            .pointer("/items")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .take(3)
                    .filter_map(|row| {
                        let path = row
                            .get("file")
                            .or_else(|| row.get("path"))
                            .and_then(Value::as_str)?;
                        Some(json!({
                            "tool": "inspect_graph",
                            "args": {
                                "source_id": args.source_id,
                                "graph": "importers",
                                "file_path": path,
                            },
                            "reason": format!("Confirm `{path}` is truly unreferenced before deleting."),
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [
                ("methodology".to_string(), json!("multi_signal")),
                ("confidence_level".to_string(), json!("medium")),
                ("actionable_next".to_string(), json!(hints)),
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
        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);
        let data = dead_code::dead_code_v2_payload(&store, min_confidence)?;
        self.success_response(&timer, data)
    }

    fn quality_signal_handler(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);

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
        if let Some(scope) = args.scope.as_deref()
            && let Some(scoped) = compute_agent_readiness_for_scope(
                &store.all_symbols_for_analysis()?,
                &store.dependency_graph_strict()?,
                store.root_path().ok().as_deref(),
                scope,
            )
        {
            // ROADMAP P2.5: the scoped readiness block used to share the
            // `agent_readiness` key with the global payload block, with
            // values scaled differently. Renaming here removes that
            // collision — global readiness stays at the top level, the
            // scoped sub-signal lives on its own key.
            obj.insert("scope".into(), json!(scope));
            // FIXUP T2.2: scoped.signal.score is f64 in [0.0, 1.0], same
            // range as its dimension siblings. Divided by 10_000.0 the
            // result became ~0.00008; remove the bogus conversion.
            obj.insert(
                "scope_agent_readiness".into(),
                json!({
                    "score": scoped.signal.score,
                    "navigability": scoped.signal.dimensions.navigability,
                    "comprehensibility": scoped.signal.dimensions.comprehensibility,
                    "verifiability": scoped.signal.dimensions.verifiability,
                    "change_safety": scoped.signal.dimensions.change_safety,
                }),
            );
            obj.insert("weakest_dimension".into(), json!(scoped.weakest_dimension));
            obj.insert(
                "top_offenders".into(),
                json!(
                    scoped
                        .top_offenders
                        .into_iter()
                        .map(|offender| json!({
                            "symbol": offender.symbol,
                            "line": offender.line,
                            "cyclomatic": offender.cyclomatic,
                            "nesting_depth": offender.nesting_depth,
                        }))
                        .collect::<Vec<_>>()
                ),
            );
            obj.insert("thresholds_failed".into(), json!(scoped.thresholds_failed));
            self.record_agent_readiness_probe(
                scope,
                scoped.signal.score,
                scoped.weakest_dimension,
                current_thresholds().agent_readiness_min,
            );
        }
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

        // ROADMAP P1.1: quality analysis — null baseline.
        let timer = MetaBuilder::new(0);

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
