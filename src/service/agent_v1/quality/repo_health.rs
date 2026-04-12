use serde_json::{Value, json};

use crate::analysis::{dead_code, quality};
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn quality_repo_health(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
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

        let dead_code_data = dead_code::dead_code_payload(&store, "file", 0.5, true, &[], None)?;
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
        let mut findings = vec![json!({
            "finding_id": "repo_health:summary",
            "analysis": "repo_health",
            "severity": "info",
            "confidence": 1.0,
            "summary": "index metadata snapshot",
            "evidence": evidence,
            "related_refs": [],
        })];
        if dead_code_pct >= 20.0 {
            let severity = if dead_code_pct >= 50.0 {
                "error"
            } else {
                "warning"
            };
            findings.push(json!({
                "finding_id": "repo_health:dead_code",
                "analysis": "repo_health",
                "severity": severity,
                "confidence": 0.6,
                "summary": format!(
                    "{dead_code_pct:.1}% of files ({dead_count}/{}) unreachable from live roots — detector may be missing entry points (check Cargo [[test]]/[[example]]/[[bench]] targets) or mod-declaration edges",
                    meta.file_count
                ),
                "evidence": { "dead_code_pct": dead_code_pct, "dead_count": dead_count, "file_count": meta.file_count },
                "related_refs": [],
            }));
        }
        if cycle_count > 0 {
            findings.push(json!({
                "finding_id": "repo_health:cycles",
                "analysis": "repo_health",
                "severity": "warning",
                "confidence": 0.8,
                "summary": format!("{cycle_count} dependency cycle(s) detected"),
                "evidence": { "cycle_count": cycle_count },
                "related_refs": [],
            }));
        }
        if index_stale {
            findings.push(json!({
                "finding_id": "repo_health:index_stale",
                "analysis": "repo_health",
                "severity": "warning",
                "confidence": 1.0,
                "summary": "index is stale compared to the filesystem; re-run ingest_sources",
                "evidence": { "indexed_at": meta.indexed_at },
                "related_refs": [],
            }));
        }
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
                "findings": findings,
            }),
        )
    }
}
