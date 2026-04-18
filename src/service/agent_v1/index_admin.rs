//! Index lifecycle + column search.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{IngestSourcesArgs, ManageIndexesArgs};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

fn invalid_request(message: String, detail: Value) -> ToolError {
    ToolError {
        category: ToolErrorCategory::InvalidArgument,
        code: "INVALID_REQUEST",
        message,
        detail,
    }
}

fn not_indexed(source_id: &str) -> ToolError {
    ToolError {
        category: ToolErrorCategory::InvalidArgument,
        code: "NOT_INDEXED",
        message: format!("No index for '{source_id}'."),
        detail: json!({ "source_id": source_id }),
    }
}

impl CodeService {
    pub(in crate::service) fn manage_indexes(&self, v: Value) -> anyhow::Result<Value> {
        let args: ManageIndexesArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let action = args.action.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match action.as_str() {
            "invalidate" | "delete" => self.manage_invalidate(&args, &action, &timer),
            "rebuild" => self.manage_rebuild(&args, &timer),
            "status" => self.manage_status(&args, &timer),
            "snapshot_quality" => self.manage_snapshot_quality(&args, &timer),
            "session_stats" => self.manage_session_stats(&timer),
            "session_end" => self.manage_session_end(&timer),
            _ => self.error_response(
                &timer,
                invalid_request(
                    format!("Unknown manage_indexes action '{}'.", args.action),
                    json!({ "action": args.action }),
                ),
            ),
        }
    }

    fn manage_invalidate(
        &self,
        args: &ManageIndexesArgs,
        action: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request(
                        "source_id required for invalidate/delete.".to_string(),
                        json!({}),
                    ),
                );
            }
        };
        let had_any = self.config.repo_dir(sid).exists() || self.config.doc_dir(sid).exists();
        if !had_any {
            return self.error_response(timer, not_indexed(sid));
        }
        self.wipe_indexes_for_repo(sid)?;
        self.source_registry().invalidate(sid);
        self.success_response(
            timer,
            json!({
                "action": action,
                "source_id": sid,
                "message": "Indexes removed for source.",
            }),
        )
    }

    fn manage_rebuild(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let root = match args.root_hint.as_deref() {
            Some(r) => r,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("root_hint required for rebuild.".to_string(), json!({})),
                );
            }
        };
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("source_id required for rebuild.".to_string(), json!({})),
                );
            }
        };
        self.wipe_indexes_for_repo(sid)?;
        self.source_registry().invalidate(sid);
        let ingest = IngestSourcesArgs {
            input_kind: "local_path".to_string(),
            domains: args.domains.clone().unwrap_or_else(|| "auto".to_string()),
            path: root.to_string(),
            incremental: false,
            enrichment: vec![],
        };
        self.ingest_sources(serde_json::to_value(ingest)?)
    }

    fn manage_status(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("source_id required for status.".to_string(), json!({})),
                );
            }
        };
        if let Ok(store) = self.ensure_repo_store(sid, None) {
            let meta = store.repo_metadata()?;
            let timer_b = MetaBuilder::new(meta.estimated_naive_tokens.saturating_mul(4));
            return self.success_response(
                &timer_b,
                json!({
                    "action": "status",
                    "source_id": sid,
                    "code_index": serde_json::to_value(&meta)?,
                }),
            );
        }
        if let Ok(dstore) = self.ensure_doc_store(sid, None) {
            let b = dstore.repo_total_bytes()?;
            let timer_b = MetaBuilder::new(b);
            let toc = dstore.toc().map(|t| t.len()).unwrap_or(0);
            return self.success_response(
                &timer_b,
                json!({
                    "action": "status",
                    "source_id": sid,
                    "doc_index": { "sections_indexed": toc },
                }),
            );
        }
        self.error_response(timer, not_indexed(sid))
    }

    fn manage_session_stats(&self, timer: &MetaBuilder) -> anyhow::Result<Value> {
        let session = self.savings_ledger_current_session()?;
        let (data, warnings) = self.session_stats_payload(&session, "session_stats")?;
        self.success_response_with_warnings(timer, data, warnings)
    }

    fn manage_session_end(&self, timer: &MetaBuilder) -> anyhow::Result<Value> {
        let session_before_end = self.savings_ledger_current_session()?;
        if let Some(source_id) = session_before_end.primary_source_id.as_deref()
            && let Ok(store) = self.ensure_repo_store(source_id, None)
            && let Ok(gov) =
                codesight_analysis::quality_signal::compute_governance_signal(&store)
            && let Ok((snapshot, marker)) =
                self.build_quality_snapshot(&store, source_id, "session_end", &gov)
            && store.save_snapshot(&snapshot).is_ok()
        {
            self.savings_ledger_set_quality_session_end(marker);
        }
        let session = self.savings_ledger_end_session()?;
        let (data, warnings) = self.session_stats_payload(&session, "session_end")?;
        self.success_response_with_warnings(timer, data, warnings)
    }

    fn manage_snapshot_quality(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let Some(ref sid) = args.source_id else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "source_id required for snapshot_quality.".to_string(),
                    detail: json!({}),
                },
            );
        };
        let store = require_store!(repo: self, sid, args.root_hint.as_deref(), timer);
        let gov = codesight_analysis::quality_signal::compute_governance_signal(&store)?;
        let snapshot_name = args.snapshot_name.as_deref().unwrap_or("session");
        let (snap, marker) = self.build_quality_snapshot(&store, sid, snapshot_name, &gov)?;
        store.save_snapshot(&snap)?;
        if snapshot_name == "session_start" {
            self.savings_ledger_set_quality_session_start(marker);
        } else if snapshot_name == "session_end" {
            self.savings_ledger_set_quality_session_end(marker);
        }
        self.success_response(
            timer,
            json!({
                "action": "snapshot_quality",
                "source_id": sid,
                "snapshot_name": snapshot_name,
                "score": gov.score,
                "structural_score": gov.structural_score,
                "agent_readiness_score": gov.agent_readiness_score,
                "confidence": gov.confidence,
                "stored": true,
            }),
        )
    }

    // ── search_columns ─────────────────────────────────────────────────────
    pub(in crate::service) fn search_columns(&self, v: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            source_id: String,
            query: String,
            #[serde(default)]
            model_pattern: Option<String>,
            #[serde(default)]
            max_results: Option<usize>,
        }
        let args: Args = ValidatedArgs::from_validated(v)?.into_inner();
        let timer = MetaBuilder::new(0);

        let store = require_store!(repo: self, &args.source_id, None, &timer);

        // Load context_metadata from meta table
        let meta_str = store.get_meta("context_metadata")?;
        let Some(meta_str) = meta_str else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "No column metadata found for this source.".into(),
                    detail: json!({}),
                },
            );
        };
        let meta: Value = serde_json::from_str(&meta_str).unwrap_or(json!({}));

        // Collect columns from all *_columns keys
        let mut all_columns: Vec<(String, String, String, String)> = Vec::new(); // (source, model, column, description)
        let mut sources: Vec<String> = Vec::new();

        if let Some(obj) = meta.as_object() {
            for (key, val) in obj {
                if !key.ends_with("_columns") {
                    continue;
                }
                let source_name = key.trim_end_matches("_columns");
                if !sources.contains(&source_name.to_string()) {
                    sources.push(source_name.to_string());
                }
                if let Some(models) = val.as_object() {
                    for (model, columns) in models {
                        // Apply model_pattern filter
                        if let Some(ref pat) = args.model_pattern {
                            let pat = pat.replace('*', "");
                            if !model.starts_with(&pat) {
                                continue;
                            }
                        }
                        if let Some(cols) = columns.as_object() {
                            for (col_name, desc) in cols {
                                all_columns.push((
                                    source_name.to_string(),
                                    model.clone(),
                                    col_name.clone(),
                                    desc.as_str().unwrap_or("").to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        if all_columns.is_empty() && sources.is_empty() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "No column metadata found for this source.".into(),
                    detail: json!({}),
                },
            );
        }

        // Score columns against query
        let query_tokens: Vec<String> = args
            .query
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .collect();

        let mut results: Vec<Value> = Vec::new();
        for (source, model, col_name, desc) in &all_columns {
            let col_lower = col_name.to_lowercase();
            let desc_lower = desc.to_lowercase();
            let mut score: i64 = 0;
            for qt in &query_tokens {
                if &col_lower == qt {
                    score += 30; // exact match
                } else if col_lower.contains(qt.as_str()) {
                    score += 15; // partial match in name
                } else if desc_lower.contains(qt.as_str()) {
                    score += 5; // match in description
                }
            }
            if score > 0 {
                results.push(json!({
                    "column": col_name,
                    "model": model,
                    "description": desc,
                    "score": score,
                    "source": source,
                }));
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b["score"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["score"].as_i64().unwrap_or(0))
        });

        // Apply max_results cap
        let max_results = args.max_results.unwrap_or(25);
        results.truncate(max_results);

        self.success_response(
            &timer,
            json!({
                "results": results,
                "result_count": results.len(),
                "sources": sources,
            }),
        )
    }

    fn session_stats_payload(
        &self,
        session: &crate::service::savings::SessionState,
        action: &str,
    ) -> anyhow::Result<(Value, Vec<Value>)> {
        let per_tool: serde_json::Map<String, Value> = session
            .per_tool
            .iter()
            .map(|(tool, stats)| {
                let mut v = json!({
                    "calls": stats.calls,
                    "tokens_saved": stats.tokens_saved,
                    "timing_ms_sum": stats.timing_ms_sum,
                    "error_count": stats.error_count,
                });
                if let Some(detail) = &stats.read_content_detail {
                    v["read_content_detail"] = json!({
                        "symbol_reads": detail.symbol_reads,
                        "section_reads": detail.section_reads,
                        "file_range_reads": detail.file_range_reads,
                        "file_full_reads": detail.file_full_reads,
                        "warning_count": detail.warning_count,
                        "bytes_returned": detail.bytes_returned,
                    });
                }
                (tool.clone(), v)
            })
            .collect();

        let calls_by_tool: serde_json::Map<String, Value> = session
            .per_tool
            .iter()
            .map(|(tool, stats)| (tool.clone(), json!(stats.calls)))
            .collect();

        let read_efficiency = per_tool
            .get("read_content")
            .and_then(|v| v.get("read_content_detail"))
            .map(|d| {
                let sym = d["symbol_reads"].as_u64().unwrap_or(0) as f64;
                let sec = d["section_reads"].as_u64().unwrap_or(0) as f64;
                let range = d["file_range_reads"].as_u64().unwrap_or(0) as f64;
                let full = d["file_full_reads"].as_u64().unwrap_or(0) as f64;
                let total = sym + sec + range + full;
                if total == 0.0 {
                    return json!({ "score": 1.0, "label": "excellent", "detail": "No reads recorded." });
                }
                let score = (sym + sec + range * 0.8) / total;
                let score = (score * 100.0).round() / 100.0;
                let label = if score >= 0.9 {
                    "excellent"
                } else if score >= 0.7 {
                    "good"
                } else if score >= 0.5 {
                    "fair"
                } else {
                    "poor"
                };
                let warns = d["warning_count"].as_u64().unwrap_or(0);
                json!({
                    "score": score,
                    "label": label,
                    "detail": format!(
                        "{}% of reads were targeted (symbol/section/range). {} whole-file read(s), {} warning(s).",
                        (score * 100.0) as u64,
                        full as u64,
                        warns
                    )
                })
            });

        let mut warnings = Vec::new();
        let quality_delta = self.session_quality_delta(session)?;
        if let Some(delta) = &quality_delta
            && delta["verdict"] == "regressed"
        {
            warnings.push(json!({
                "code": "QUALITY_REGRESSION",
                "message": "session quality delta regressed versus the session baseline.",
                "quality_delta": delta,
            }));
        }

        let mut anti_patterns = Vec::new();
        let blast_radius_calls = session
            .evaluate_impact_usage
            .get("blast_radius")
            .copied()
            .unwrap_or(0);
        let path_rename_calls = session
            .evaluate_impact_usage
            .get("path_rename")
            .copied()
            .unwrap_or(0);
        if path_rename_calls > 0 && blast_radius_calls == 0 {
            anti_patterns.push(json!({
                "code": "REFACTOR_WITHOUT_BLAST_RADIUS",
                "message": "path_rename was used without a preceding blast_radius call.",
            }));
        }
        if blast_radius_calls == 0
            && session
                .use_prefix_edits
                .values()
                .copied()
                .any(|count| count >= 5)
        {
            anti_patterns.push(json!({
                "code": "HAND_ROLLED_PATH_RENAME",
                "message": "Edit/Write hand-rewrote use-prefix imports across 5+ files with no blast_radius call. Prefer evaluate_impact analysis=path_rename.",
                "use_prefix_edits": session.use_prefix_edits,
            }));
        }

        let mut data = json!({
            "action": action,
            "session_id": session.id,
            "started_at": session.started_at,
            "ended_at": session.ended_at,
            "calls_by_tool": calls_by_tool,
            "per_tool": per_tool,
            "tokens_saved_total": session.tokens_saved_total,
            "fallbacks_to_grep_or_bash": session.fallbacks_to_grep_or_bash,
            "evaluate_impact_usage": session.evaluate_impact_usage,
            "stale_warnings_fired_with_in_flight_paths": session.stale_warnings_fired_with_in_flight_paths,
            "primary_source_id": session.primary_source_id,
            "use_prefix_edits": session.use_prefix_edits,
            "anti_patterns": anti_patterns,
            "edits_after_codesight_read": session.edits_after_codesight_read,
            "codesight_read_paths_tracked": session.codesight_read_paths.len(),
        });
        if let Some(eff) = read_efficiency {
            data["read_efficiency"] = eff;
        }
        if let Some(delta) = quality_delta {
            data["quality_delta"] = delta;
        }
        Ok((data, warnings))
    }

    fn session_quality_delta(
        &self,
        session: &crate::service::savings::SessionState,
    ) -> anyhow::Result<Option<Value>> {
        let Some(start) = session.quality_session_start.as_ref() else {
            return Ok(None);
        };
        let store = match self.ensure_repo_store(&start.source_id, None) {
            Ok(store) => store,
            Err(_) => return Ok(None),
        };
        let Some(baseline) = store.load_snapshot(&start.source_id, &start.snapshot_name)? else {
            return Ok(None);
        };
        let baseline_avg_complexity = start.avg_complexity.unwrap_or(baseline.avg_complexity);
        let baseline_hotspots = start.hotspots.clone();
        let (after_score, after_structural, after_agent, after_dimensions, after_avg_complexity, after_hotspots) =
            if let Some(end) = session.quality_session_end.as_ref() {
                match store.load_snapshot(&end.source_id, &end.snapshot_name)? {
                    Some(snapshot) => (
                        snapshot.score,
                        snapshot.structural_score,
                        snapshot.agent_readiness_score,
                        json!({
                            "navigability": snapshot.navigability,
                            "comprehensibility": snapshot.comprehensibility,
                            "verifiability": snapshot.verifiability,
                            "change_safety": snapshot.change_safety,
                        }),
                        end.avg_complexity.unwrap_or(snapshot.avg_complexity),
                        end.hotspots.clone(),
                    ),
                    None => return Ok(None),
                }
            } else {
                let gov = match codesight_analysis::quality_signal::compute_governance_signal(&store) {
                    Ok(gov) => gov,
                    Err(_) => return Ok(None),
                };
                let (avg_complexity, hotspots) = match self.quality_snapshot_metrics(&store) {
                    Ok(metrics) => metrics,
                    Err(_) => return Ok(None),
                };
                (
                    gov.score,
                    gov.structural_score,
                    gov.agent_readiness_score,
                    json!({
                        "navigability": gov.agent.dimensions.navigability,
                        "comprehensibility": gov.agent.dimensions.comprehensibility,
                        "verifiability": gov.agent.dimensions.verifiability,
                        "change_safety": gov.agent.dimensions.change_safety,
                    }),
                    avg_complexity,
                    hotspots,
                )
            };

        let per_dimension = json!({
            "navigability": dimension_delta(baseline.navigability, after_dimensions["navigability"].as_f64()),
            "comprehensibility": dimension_delta(
                baseline.comprehensibility,
                after_dimensions["comprehensibility"].as_f64()
            ),
            "verifiability": dimension_delta(baseline.verifiability, after_dimensions["verifiability"].as_f64()),
            "change_safety": dimension_delta(baseline.change_safety, after_dimensions["change_safety"].as_f64()),
        });

        let agent_before = baseline.agent_readiness_score as f64 / 10_000.0;
        let agent_after = after_agent as f64 / 10_000.0;
        let structural_before = baseline.structural_score as f64 / 10_000.0;
        let structural_after = after_structural as f64 / 10_000.0;
        let overall_before = baseline.score as f64 / 10_000.0;
        let overall_after = after_score as f64 / 10_000.0;
        let (new_hotspots, resolved_hotspots, largest_regression, largest_improvement) =
            hotspot_deltas(&baseline_hotspots, &after_hotspots);
        let weakest_regression = [
            per_dimension["navigability"]["delta"].as_f64().unwrap_or(0.0),
            per_dimension["comprehensibility"]["delta"].as_f64().unwrap_or(0.0),
            per_dimension["verifiability"]["delta"].as_f64().unwrap_or(0.0),
            per_dimension["change_safety"]["delta"].as_f64().unwrap_or(0.0),
        ]
        .into_iter()
        .fold(0.0, f64::min);
        let avg_complexity_regressed = after_avg_complexity > baseline_avg_complexity + 0.5;
        let avg_complexity_improved = after_avg_complexity + 0.5 < baseline_avg_complexity;
        let hotspot_regressed = largest_regression >= 2;
        let hotspot_improved = largest_improvement >= 2;
        let verdict = if overall_after + 0.01 < overall_before
            || weakest_regression < -0.05
            || avg_complexity_regressed
            || hotspot_regressed
        {
            "regressed"
        } else if overall_after > overall_before + 0.01
            || avg_complexity_improved
            || hotspot_improved
        {
            "improved"
        } else {
            "neutral"
        };

        Ok(Some(json!({
            "agent_readiness_score": score_delta(agent_before, agent_after),
            "structural_score": score_delta(structural_before, structural_after),
            "score": score_delta(overall_before, overall_after),
            "avg_complexity": score_delta(baseline_avg_complexity, after_avg_complexity),
            "per_dimension": per_dimension,
            "new_hotspots": new_hotspots,
            "resolved_hotspots": resolved_hotspots,
            "verdict": verdict,
        })))
    }
}

fn score_delta(before: f64, after: f64) -> Value {
    json!({
        "before": before,
        "after": after,
        "delta": ((after - before) * 100.0).round() / 100.0,
    })
}

fn dimension_delta(before: Option<f64>, after: Option<f64>) -> Value {
    match (before, after) {
        (Some(before), Some(after)) => score_delta(before, after),
        _ => Value::Null,
    }
}

fn hotspot_deltas(
    before: &[crate::service::savings::QualityHotspotMarker],
    after: &[crate::service::savings::QualityHotspotMarker],
) -> (Vec<Value>, Vec<Value>, u64, u64) {
    let before_by_file: std::collections::HashMap<&str, u64> = before
        .iter()
        .map(|entry| (entry.file.as_str(), entry.cyclomatic))
        .collect();
    let after_by_file: std::collections::HashMap<&str, u64> = after
        .iter()
        .map(|entry| (entry.file.as_str(), entry.cyclomatic))
        .collect();

    let mut regressions = Vec::new();
    let mut largest_regression = 0_u64;
    for entry in after {
        let before_cyclomatic = before_by_file.get(entry.file.as_str()).copied().unwrap_or(0);
        if entry.cyclomatic > before_cyclomatic {
            let delta = entry.cyclomatic - before_cyclomatic;
            largest_regression = largest_regression.max(delta);
            regressions.push(json!({
                "file": entry.file,
                "cyclomatic": entry.cyclomatic,
                "before_cyclomatic": before_cyclomatic,
                "delta": delta,
            }));
        }
    }
    regressions.sort_by(|left, right| {
        right["delta"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&left["delta"].as_u64().unwrap_or(0))
            .then_with(|| {
                left["file"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["file"].as_str().unwrap_or_default())
            })
    });

    let mut resolved = Vec::new();
    let mut largest_improvement = 0_u64;
    for entry in before {
        let after_cyclomatic = after_by_file.get(entry.file.as_str()).copied().unwrap_or(0);
        if entry.cyclomatic > after_cyclomatic {
            let delta = entry.cyclomatic - after_cyclomatic;
            largest_improvement = largest_improvement.max(delta);
            resolved.push(json!({
                "file": entry.file,
                "before_cyclomatic": entry.cyclomatic,
                "after_cyclomatic": after_cyclomatic,
                "delta": delta,
            }));
        }
    }
    resolved.sort_by(|left, right| {
        right["delta"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&left["delta"].as_u64().unwrap_or(0))
            .then_with(|| {
                left["file"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["file"].as_str().unwrap_or_default())
            })
    });

    regressions.truncate(10);
    resolved.truncate(10);
    (regressions, resolved, largest_regression, largest_improvement)
}
