//! `evaluate_quality analysis=refactor_targets` — high-complexity,
//! high-fan-in candidates with truncated caller-file listing.

use rustc_hash::{FxHashMap, FxHashSet};

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use codesight_analysis::complexity::cyclomatic_complexity_for_path;
use codesight_analysis::quality;

impl CodeService {
    pub(super) fn quality_refactor_targets(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let min_complexity = args.min_complexity.unwrap_or(0);
        let min_callers = args.min_callers.unwrap_or(0);
        let top_n = args.top_n.unwrap_or(50);
        let include_conventional = args.include_conventional.unwrap_or(false);
        let top_caller_files = args.top_caller_files.unwrap_or(10).clamp(1, 50);

        // ROADMAP P1.1: quality analysis — null baseline.
        // Aggregates complexity (safe per-symbol) + caller heuristics — partial.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);

        let all_symbols = store.all_symbols_cached()?;

        let scope_files: Vec<String> = if let Some(file_path) = args.file_path.as_deref() {
            let all_files = store.all_file_paths()?;
            let Some(resolved_path) = all_files
                .iter()
                .find(|f| {
                    f.as_str() == file_path
                        || f.ends_with(&format!("/{file_path}"))
                        || f.ends_with(file_path)
                })
                .cloned()
            else {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("File '{file_path}' not found in index."),
                        detail: json!({ "file_path": file_path }),
                    },
                );
            };
            vec![resolved_path]
        } else {
            let top_files = args.top_files.unwrap_or(10).max(1);
            let root_path = store.root_path().ok();
            let file_churn = root_path
                .as_ref()
                .map(|root| codesight_analysis::agent_readiness::compute_file_churn(root))
                .unwrap_or_default();
            quality::top_hotspot_files_from_cached(
                &all_symbols,
                &file_churn,
                top_files,
                include_conventional,
            )
        };

        let scope_tag = if args.file_path.is_some() {
            "file"
        } else {
            "repository"
        };
        let file_set: FxHashSet<&str> = scope_files.iter().map(String::as_str).collect();

        struct Cand<'a> {
            sym: &'a codesight_storage::storage::AnalysisSymbol,
            cyclomatic: u64,
        }
        let mut cands: Vec<Cand<'_>> = Vec::new();
        for sym in &*all_symbols {
            if !file_set.contains(sym.file_path.as_str()) {
                continue;
            }
            if !include_conventional
                && codesight_language_support::is_conventional_path(&sym.file_path)
            {
                continue;
            }
            let kind_lower = sym.kind.to_ascii_lowercase();
            if kind_lower != "function" && kind_lower != "method" {
                continue;
            }
            let cyclomatic =
                cyclomatic_complexity_for_path(&sym.source_text, &sym.file_path) as u64;
            if cyclomatic < min_complexity {
                continue;
            }
            cands.push(Cand { sym, cyclomatic });
        }

        let mut name_to_cands: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
        for (ci, cand) in cands.iter().enumerate() {
            if cand.sym.name.len() < 2 {
                continue;
            }
            name_to_cands
                .entry(cand.sym.name.as_str())
                .or_default()
                .push(ci);
        }

        let mut counts: Vec<usize> = vec![0; cands.len()];
        let mut files: Vec<FxHashSet<&str>> = vec![FxHashSet::default(); cands.len()];
        let mut seen_in_other: FxHashSet<usize> = FxHashSet::default();
        for other in &*all_symbols {
            if name_to_cands.is_empty() {
                break;
            }
            seen_in_other.clear();
            for ident in other
                .source_text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|s| !s.is_empty())
            {
                if let Some(indices) = name_to_cands.get(ident) {
                    for &ci in indices {
                        if cands[ci].sym.id == other.id {
                            continue;
                        }
                        if !seen_in_other.insert(ci) {
                            continue;
                        }
                        counts[ci] += 1;
                        files[ci].insert(other.file_path.as_str());
                    }
                }
            }
        }

        let mut candidates: Vec<Value> = Vec::new();
        for (ci, cand) in cands.iter().enumerate() {
            let caller_count = counts[ci];
            if (caller_count as u64) < min_callers {
                continue;
            }
            let caller_files_total = files[ci].len();
            let mut sorted_callers: Vec<&str> = files[ci].iter().copied().collect();
            sorted_callers.sort_by(|a, b| {
                let depth_a = a.matches('/').count();
                let depth_b = b.matches('/').count();
                depth_a.cmp(&depth_b).then_with(|| a.cmp(b))
            });
            let truncated = sorted_callers.len() > top_caller_files;
            if truncated {
                sorted_callers.truncate(top_caller_files);
            }

            let score = cand.cyclomatic as f64 * (1.0 + caller_count as f64);
            let mut entry = json!({
                "id": cand.sym.id,
                "name": cand.sym.name,
                "kind": cand.sym.kind,
                "file": cand.sym.file_path,
                "line": cand.sym.start_line,
                "cyclomatic": cand.cyclomatic,
                "caller_count": caller_count,
                "caller_files": sorted_callers,
                "score": score,
            });
            if truncated && let Value::Object(map) = &mut entry {
                map.insert("caller_files_total".to_string(), json!(caller_files_total));
            }
            candidates.push(entry);
        }

        candidates.sort_by(|a, b| {
            let sa = a["score"].as_f64().unwrap_or(0.0);
            let sb = b["score"].as_f64().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(top_n);

        let mut data = json!({
            "analysis": "refactor_targets",
            "scope": scope_tag,
            "files_scanned": scope_files.len(),
            "candidates": candidates,
        });
        if args.file_path.is_some()
            && let Value::Object(ref mut map) = data
        {
            map.insert("file".to_string(), json!(scope_files[0]));
        }
        self.success_response(&timer, data)
    }
}
