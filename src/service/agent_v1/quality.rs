//! Quality / complexity / docs analyses.

use serde_json::{Value, json};

use crate::analysis::{dead_code, quality};
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::args::parse_args;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = parse_args(v)?;
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match a.as_str() {
            "complexity" => {
                let Some(r) = args.refs.as_ref().and_then(|x| x.first()) else {
                    return self.error_response(
                        &timer,
                        ToolError {
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
                let unstable_modules = usize::from(meta.is_stale.unwrap_or(false));
                let summary = format!(
                    "{} indexed files, {} symbols ({} functions/methods); index v{}",
                    meta.file_count, meta.symbol_count, fn_method, meta.index_version
                );
                let evidence = serde_json::to_value(&meta)?;
                let warnings = vec![json!({
                    "code": "PARTIAL_REPO_HEALTH",
                    "message": "dead_code_pct, dead_count, cycle_count, cycles_sample, and top_hotspots are placeholders until dependency/dead-code analysis is implemented.",
                })];
                self.success_response_with_warnings(
                    &timer,
                    json!({
                        "analysis": "repo_health",
                        "repo": meta.repo,
                        "summary": summary,
                        "total_files": meta.file_count,
                        "total_symbols": meta.symbol_count,
                        "avg_complexity": avg_complexity,
                        "dead_code_pct": 0.0,
                        "dead_count": 0,
                        "cycle_count": 0,
                        "cycles_sample": json!([]),
                        "unstable_modules": unstable_modules,
                        "top_hotspots": json!([]),
                        "fn_method_count": fn_method,
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
                    warnings,
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
