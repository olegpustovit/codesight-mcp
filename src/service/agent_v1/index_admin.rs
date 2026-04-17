//! Index lifecycle + column search.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{IngestSourcesArgs, ManageIndexesArgs};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn manage_indexes(&self, v: Value) -> anyhow::Result<Value> {
        let args: ManageIndexesArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let action = args.action.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match action.as_str() {
            "invalidate" | "delete" => {
                let Some(ref sid) = args.source_id else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "source_id required for invalidate/delete.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let had_any =
                    self.config.repo_dir(sid).exists() || self.config.doc_dir(sid).exists();
                if !had_any {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "NOT_INDEXED",
                            message: format!("No index for '{sid}'."),
                            detail: json!({ "source_id": sid }),
                        },
                    );
                }
                self.wipe_indexes_for_repo(sid)?;
                self.success_response(
                    &timer,
                    json!({
                        "action": action,
                        "source_id": sid,
                        "message": "Indexes removed for source.",
                    }),
                )
            }
            "rebuild" => {
                let Some(ref root) = args.root_hint else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "root_hint required for rebuild.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let Some(ref sid) = args.source_id else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "source_id required for rebuild.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                self.wipe_indexes_for_repo(sid)?;
                let dom = args.domains.clone().unwrap_or_else(|| "auto".to_string());
                let ingest = IngestSourcesArgs {
                    input_kind: "local_path".to_string(),
                    domains: dom,
                    path: root.clone(),
                    incremental: false,
                    enrichment: vec![],
                };
                self.ingest_sources(serde_json::to_value(ingest)?)
            }
            "status" => {
                let Some(ref sid) = args.source_id else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "source_id required for status.".to_string(),
                            detail: json!({}),
                        },
                    );
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
                self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "NOT_INDEXED",
                        message: format!("No index for '{sid}'."),
                        detail: json!({ "source_id": sid }),
                    },
                )
            }
            "snapshot_quality" => self.manage_snapshot_quality(&args, &timer),
            "session_stats" => {
                let session = self.savings_ledger_current_session()?;
                let per_tool: serde_json::Map<String, Value> = session
                    .per_tool
                    .into_iter()
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
                        (tool, v)
                    })
                    .collect();

                // Compute read efficiency score from read_content detail.
                let read_efficiency = per_tool.get("read_content")
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
                        // Weighted: symbol=1.0, section=1.0, file_range=0.8, file_full=0.0
                        let score = (sym * 1.0 + sec * 1.0 + range * 0.8) / total;
                        let score = (score * 100.0).round() / 100.0;
                        let label = if score >= 0.9 { "excellent" }
                            else if score >= 0.7 { "good" }
                            else if score >= 0.5 { "fair" }
                            else { "poor" };
                        let full_u = full as u64;
                        let warns = d["warning_count"].as_u64().unwrap_or(0);
                        json!({
                            "score": score,
                            "label": label,
                            "detail": format!(
                                "{}% of reads were targeted (symbol/section/range). {} whole-file read(s), {} warning(s).",
                                (score * 100.0) as u64, full_u, warns
                            )
                        })
                    });

                let mut data = json!({
                    "action": "session_stats",
                    "started_at": session.started_at,
                    "per_tool": per_tool,
                });
                if let Some(eff) = read_efficiency {
                    data["read_efficiency"] = eff;
                }
                self.success_response(&timer, data)
            }
            _ => self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown manage_indexes action '{}'.", args.action),
                    detail: json!({ "action": args.action }),
                },
            ),
        }
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
        let snap = codesight_storage::storage::QualitySnapshot {
            source_id: sid.clone(),
            name: snapshot_name.to_string(),
            created_at: String::new(), // filled by DEFAULT in SQLite
            score: gov.score,
            structural_score: gov.structural_score,
            agent_readiness_score: gov.agent_readiness_score,
            modularity: gov.structural.root_causes.modularity,
            acyclicity: Some(gov.structural.root_causes.acyclicity),
            depth: gov.structural.root_causes.depth,
            equality: Some(gov.structural.root_causes.equality),
            redundancy: Some(gov.structural.root_causes.redundancy),
            navigability: Some(gov.agent.dimensions.navigability),
            comprehensibility: Some(gov.agent.dimensions.comprehensibility),
            verifiability: Some(gov.agent.dimensions.verifiability),
            change_safety: Some(gov.agent.dimensions.change_safety),
            cycle_count: gov.structural.coverage.cycle_count as u32,
            dead_count: gov.structural.coverage.dead_files as u32,
            dup_count: gov.structural.coverage.duplicate_functions as u32,
            file_count: gov.structural.coverage.graph_files as u32,
            symbol_count: gov.structural.coverage.functions_with_source as u32,
            confidence: gov.confidence.to_string(),
            avg_complexity: 0.0, // not tracked separately
        };
        store.save_snapshot(&snap)?;
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
}
