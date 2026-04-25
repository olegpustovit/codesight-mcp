//! Session lifecycle actions for `manage_indexes`: session_stats,
//! session_end, snapshot_quality — plus the shared session_stats payload
//! builder and quality-delta computation.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::ManageIndexesArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use crate::service::savings::{QualityHotspotMarker, SessionState};

impl CodeService {
    pub(super) fn manage_session_stats(&self, timer: &MetaBuilder) -> anyhow::Result<Value> {
        let session = self.savings_ledger_current_session()?;
        let (data, warnings) = self.session_stats_payload(&session, "session_stats")?;
        self.success_response_with_warnings(timer, data, warnings)
    }

    pub(super) fn manage_session_end(&self, timer: &MetaBuilder) -> anyhow::Result<Value> {
        let session_before_end = self.savings_ledger_current_session()?;
        if let Some(source_id) = session_before_end.primary_source_id.as_deref()
            && let Ok(store) = self.ensure_repo_store(source_id, None)
            && let Ok(gov) = codesight_analysis::quality_signal::compute_governance_signal(&store)
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

    pub(super) fn manage_snapshot_quality(
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

    fn session_stats_payload(
        &self,
        session: &SessionState,
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
            // FIXUP T4.1: every high-signal warning carries the same
            // {code, message, detail, suggested_tool} envelope so callers
            // can act on them uniformly. `detail` carries the delta;
            // `suggested_tool` points at the regression-check follow-up.
            let suggested_tool = session.primary_source_id.as_ref().map(|sid| {
                json!({
                    "tool": "evaluate_quality",
                    "args": {
                        "source_id": sid,
                        "analysis": "regression_check",
                        "snapshot_name": "session_start",
                    },
                })
            });
            let mut warn = json!({
                "code": "QUALITY_REGRESSION",
                "message": "session quality delta regressed versus the session baseline.",
                "detail": { "quality_delta": delta },
            });
            if let Some(st) = suggested_tool
                && let Value::Object(ref mut m) = warn
            {
                m.insert("suggested_tool".to_string(), st);
            }
            warnings.push(warn);
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
            "blocked_grep_bash_attempts": session.blocked_grep_bash_attempts,
            "evaluate_impact_usage": session.evaluate_impact_usage,
            "stale_warnings_in_flight": session.stale_warnings_in_flight,
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

    fn session_quality_delta(&self, session: &SessionState) -> anyhow::Result<Option<Value>> {
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
        let (
            after_score,
            after_structural,
            after_agent,
            after_dimensions,
            after_avg_complexity,
            after_hotspots,
        ) = if let Some(end) = session.quality_session_end.as_ref() {
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

        // FIXUP T2.2: snapshot scores are persisted as f64 in [0.0, 1.0]
        // (GovernanceSignal.score's doc comment is explicit on this), so no
        // unit conversion is needed. The prior `/ 10_000.0` was the same
        // bogus conversion found in quality/mod.rs and produced deltas in
        // the 1e-5 range.
        let agent_before = baseline.agent_readiness_score;
        let agent_after = after_agent;
        let structural_before = baseline.structural_score;
        let structural_after = after_structural;
        let overall_before = baseline.score;
        let overall_after = after_score;
        let (new_hotspots, resolved_hotspots, largest_regression, largest_improvement) =
            hotspot_deltas(&baseline_hotspots, &after_hotspots);
        let weakest_regression = [
            per_dimension["navigability"]["delta"]
                .as_f64()
                .unwrap_or(0.0),
            per_dimension["comprehensibility"]["delta"]
                .as_f64()
                .unwrap_or(0.0),
            per_dimension["verifiability"]["delta"]
                .as_f64()
                .unwrap_or(0.0),
            per_dimension["change_safety"]["delta"]
                .as_f64()
                .unwrap_or(0.0),
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
    before: &[QualityHotspotMarker],
    after: &[QualityHotspotMarker],
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
        let before_cyclomatic = before_by_file
            .get(entry.file.as_str())
            .copied()
            .unwrap_or(0);
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
    (
        regressions,
        resolved,
        largest_regression,
        largest_improvement,
    )
}
