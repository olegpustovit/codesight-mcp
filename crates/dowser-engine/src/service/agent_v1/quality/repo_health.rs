use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use dowser_analysis::{quality, quality_signal};

use super::cached_default_dead_code_count;

impl CodeService {
    pub(super) fn quality_repo_health(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let mut meta = store.repo_metadata()?;
        if let Ok(root) = store.root_path() {
            let indexer = self.indexer();
            meta.is_stale = store
                .is_stale(&root, |p| indexer.detect_language(&root, p).is_some())
                .ok();
        }
        // ROADMAP P1.1: repo_health is a structural overview — null baseline.
        // Combines authoritative index lookups with heuristic dead_code +
        // hotspot signals — partial.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);
        let fn_method = store.fn_method_symbol_count().unwrap_or(0);
        let avg_complexity = store.avg_line_span_fn_methods().unwrap_or(0.0);
        let index_stale = meta.is_stale.unwrap_or(false);

        let adj = store.dependency_graph_strict()?;
        let cycles = dowser_analysis::graph::find_cycles(&adj);
        let cycle_count = cycles.len();
        let cycles_sample: Vec<_> = cycles.iter().take(5).cloned().collect();

        // Tier 5.1: use the cached dead-code count instead of running a
        // fresh reachability walk. `repo_health` warms in < 1 s; a cold
        // cache returns `dead_count_pending=true` and the caller can run
        // `health analysis=dead_code` separately for a fresh number.
        let cached_dead = cached_default_dead_code_count(self, &args.source_id);
        let dead_count = cached_dead.unwrap_or(0) as usize;
        let dead_count_pending = cached_dead.is_none();
        let dead_code_pct = if meta.file_count == 0 || dead_count_pending {
            0.0
        } else {
            (dead_count as f64 / meta.file_count as f64) * 100.0
        };

        let hotspots_data = quality::hotspots_payload(&store, 1, 5, false)?;
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

        // Embed full governance signal (structural + agent readiness).
        // FIXUP T2.6: gov.{score,structural_score,agent_readiness_score} are
        // already normalized to `[0.0, 1.0]` — render them as percents and
        // drop the fabricated `/10000` composite that was never meaningful.
        if let Ok(gov) = quality_signal::compute_governance_signal(&store) {
            let suggestion =
                quality_signal::suggestion_for_bottleneck(gov.bottleneck, &gov.structural.coverage);
            let evidence = quality_signal::governance_signal_json(&gov, &suggestion);
            findings.push(json!({
                "finding_id": "repo_health:quality_signal",
                "analysis": "repo_health",
                "severity": "info",
                "confidence": 1.0,
                "summary": format!(
                    "Structural: {:.0}%. Agent readiness: {:.0}%. Bottleneck: {} ({}).",
                    gov.structural_score * 100.0,
                    gov.agent_readiness_score * 100.0,
                    gov.bottleneck,
                    gov.bottleneck_layer,
                ),
                "evidence": evidence,
                "related_refs": [],
            }));
        }

        if dead_count_pending {
            findings.push(json!({
                "finding_id": "repo_health:dead_code",
                "analysis": "repo_health",
                "severity": "info",
                "confidence": 0.0,
                "summary": "dead-code count not in cache — run `health analysis=dead_code` for a fresh figure",
                "evidence": { "pending": true },
                "related_refs": [],
            }));
        } else if dead_code_pct >= 20.0 {
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
                "dead_count_pending": dead_count_pending,
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
