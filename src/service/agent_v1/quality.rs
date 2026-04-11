//! Quality / complexity / docs analyses.

use serde_json::{Value, json};

use crate::analysis::{dead_code, quality};
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match a.as_str() {
            "complexity" => {
                let Some(r) = args.refs.as_ref().and_then(|x| x.first()) else {
                    return self.error_response(
                        &timer,
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
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "complexity requires a symbol ref.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };
                let data = match quality::complexity_payload(&store, &id)? {
                    Some(data) => data,
                    None => {
                        return self.error_response(
                            &timer,
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
            "repo_health" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };
                let mut meta = store.repo_metadata()?;
                if let Ok(root) = store.root_path() {
                    let extractors = self.extractors_for_root(&root);
                    meta.is_stale = store
                        .is_stale(&root, |p| extractors.extractor_for_path(p).is_some())
                        .ok();
                }
                let raw_bytes = meta.estimated_naive_tokens.saturating_mul(4);
                let timer = MetaBuilder::new(raw_bytes);
                let fn_method = store.fn_method_symbol_count().unwrap_or(0);
                let avg_complexity = store.avg_line_span_fn_methods().unwrap_or(0.0);
                let index_stale = meta.is_stale.unwrap_or(false);

                let adj = store.dependency_graph_strict()?;
                let cycles = crate::analysis::graph::find_cycles(&adj);
                let cycle_count = cycles.len();
                let cycles_sample: Vec<_> = cycles.iter().take(5).cloned().collect();

                let dead_code_data = dead_code::dead_code_payload(&store, "file", 0.5, true, &[])?;
                let dead_count = dead_code_data
                    .get("dead_file_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let dead_code_pct = if meta.file_count == 0 {
                    0.0
                } else {
                    (dead_count as f64 / meta.file_count as f64) * 100.0
                };

                let hotspots_data = quality::hotspots_payload(&store, 1, 5)?;
                let top_hotspots = hotspots_data
                    .get("hotspots")
                    .cloned()
                    .unwrap_or_else(|| json!([]));

                let summary = format!(
                    "{} indexed files, {} symbols ({} functions/methods); index v{}",
                    meta.file_count, meta.symbol_count, fn_method, meta.index_version
                );
                let evidence = serde_json::to_value(&meta)?;
                self.success_response(
                    &timer,
                    json!({
                        "analysis": "repo_health",
                        "repo": meta.repo,
                        "summary": summary,
                        "total_files": meta.file_count,
                        "total_symbols": meta.symbol_count,
                        "fn_method_count": fn_method,
                        "avg_complexity": avg_complexity,
                        "dead_count": dead_count,
                        "dead_code_pct": dead_code_pct,
                        "cycle_count": cycle_count,
                        "cycles_sample": cycles_sample,
                        "top_hotspots": top_hotspots,
                        "index_stale": index_stale,
                        "findings": [{
                            "finding_id": "repo_health:summary",
                            "analysis": "repo_health",
                            "severity": "info",
                            "confidence": 1.0,
                            "summary": "index metadata snapshot",
                            "evidence": evidence,
                            "related_refs": [],
                        }],
                    }),
                )
            }
            "dead_code" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };
                let granularity = args.granularity.as_deref().unwrap_or("file");
                let min_confidence = args.min_confidence.unwrap_or(0.5);
                let include_tests = args.include_tests.unwrap_or(true);
                let entry_patterns = args.entry_point_patterns.clone().unwrap_or_default();
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let data = dead_code::dead_code_payload(
                    &store,
                    granularity,
                    min_confidence,
                    include_tests,
                    &entry_patterns,
                )?;
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
            "dead_code_v2" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };
                let min_confidence = args.min_confidence.unwrap_or(0.5);
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let data = dead_code::dead_code_v2_payload(&store, min_confidence)?;
                self.success_response(&timer, data)
            }
            tail => self.evaluate_quality_tail(&args, tail, &timer),
        }
    }
}
