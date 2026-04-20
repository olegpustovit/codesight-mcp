//! Quality analyses: hotspots, extraction, links, doc coverage.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

static MD_INTERNAL_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Infallible: fixed markdown link pattern.
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").expect("infallible markdown link regex")
});

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::complexity::cyclomatic_complexity;
use codesight_analysis::quality;

impl CodeService {
    pub(in crate::service) fn evaluate_quality_tail(
        &self,
        args: &EvaluateQualityArgs,
        analysis: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        match analysis {
            "hotspots" => self.quality_hotspots(args, timer),
            "refactor_targets" => self.quality_refactor_targets(args, timer),
            "broken_links" => self.quality_broken_links(args, timer),
            "doc_coverage" => self.quality_doc_coverage(args, timer),
            "test_gaps" => self.quality_test_gaps(args, timer),
            _ => self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown quality analysis '{analysis}'."),
                    detail: json!({ "analysis": analysis }),
                },
            ),
        }
    }

    fn quality_hotspots(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let min_complexity = args.min_complexity.unwrap_or(0);
        let top_n = args.top_n.unwrap_or(50);
        let include_conventional = args.include_conventional.unwrap_or(false);

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let data =
            quality::hotspots_payload(&store, min_complexity, top_n, include_conventional)?;

        // `methodology` + `confidence_level` are fixed per-tool constants —
        // belong in the tool docs, not repeated in every response envelope
        // (PLAN2 §3.3d).
        self.success_response(&timer, data)
    }

    fn quality_refactor_targets(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let min_complexity = args.min_complexity.unwrap_or(0);
        let min_callers = args.min_callers.unwrap_or(0);
        let top_n = args.top_n.unwrap_or(50);
        let include_conventional = args.include_conventional.unwrap_or(false);
        // FEEDBACK.md §4b: cap caller_files per candidate. Max 50.
        let top_caller_files = args
            .top_caller_files
            .unwrap_or(10)
            .clamp(1, 50);

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);

        // One symbol load per request, shared with every downstream sweep
        // (candidate filter + reference tokenizer + scope-file ranking).
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
            // Rank in-process over the cached symbol vec — saves a second
            // `all_symbols_for_analysis` round trip + a repeat cyclomatic
            // pass vs. calling `hotspots_payload`.
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
        let file_set: HashSet<&str> = scope_files.iter().map(String::as_str).collect();

        // Pass 1: filter to candidate functions/methods in scope. Indexed by
        // position in `cands` so all downstream state (`counts`, `files`) can
        // use Vec<usize> instead of HashMap<&str, _>.
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
            let cyclomatic = cyclomatic_complexity(&sym.source_text) as u64;
            if cyclomatic < min_complexity {
                continue;
            }
            cands.push(Cand { sym, cyclomatic });
        }

        // Pass 2: tokenize each source_text into identifiers once and look
        // them up in a `name → cand-indices` map. Replaces an
        // O(all × cands × contains) substring sweep with O(total_source_bytes).
        // Mirror `crates/codesight-analysis/src/graph/calls.rs` so call-graph
        // and reference-count paths share the same cost model.
        let mut name_to_cands: HashMap<&str, Vec<usize>> = HashMap::new();
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
        let mut files: Vec<HashSet<&str>> = vec![HashSet::new(); cands.len()];
        let mut seen_in_other: HashSet<usize> = HashSet::new();
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
            // FEEDBACK.md §4b: deterministic truncation — shortest path depth
            // first, then alphabetical. This keeps the N "easiest to skim"
            // caller files visible and gives the agent a stable signal
            // across runs.
            let caller_files_total = files[ci].len();
            let mut sorted_callers: Vec<&str> =
                files[ci].iter().copied().collect();
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
            if truncated
                && let Value::Object(map) = &mut entry
            {
                map.insert(
                    "caller_files_total".to_string(),
                    json!(caller_files_total),
                );
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

    fn quality_broken_links(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let doc_store =
            require_store!(doc: self, &args.source_id, args.repo_path.as_deref(), timer);
        let all_sections = doc_store.all_sections()?;
        let all_files: HashSet<String> = doc_store.all_doc_files()?.into_iter().collect();

        let mut file_headings: HashMap<String, HashSet<String>> = HashMap::new();
        for sec in &all_sections {
            file_headings
                .entry(sec.file_path.clone())
                .or_default()
                .insert(sec.heading.to_lowercase().replace(' ', "-"));
        }

        let mut broken_links: Vec<Value> = Vec::new();
        for sec in &all_sections {
            for cap in MD_INTERNAL_LINK_RE.captures_iter(&sec.content_text) {
                let Some(target) = cap.get(2).map(|m| m.as_str()) else {
                    continue;
                };
                if let Some(entry) = check_internal_link(sec, target, &all_files, &file_headings) {
                    broken_links.push(entry);
                }
            }
        }

        self.success_response(
            timer,
            json!({
                "analysis": "broken_links",
                "repo": args.source_id,
                "docs_scanned": all_files.len(),
                "sections_scanned": all_sections.len(),
                "broken_link_count": broken_links.len(),
                "broken_links": broken_links,
            }),
        )
    }

    fn quality_doc_coverage(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let doc_store =
            require_store!(doc: self, &args.source_id, args.repo_path.as_deref(), timer);

        let mut symbol_ids: Vec<String> = args
            .refs
            .as_ref()
            .map(|refs| {
                refs.iter()
                    .filter(|r| r.kind == "symbol")
                    .filter_map(|r| r.stable_id.clone().or(r.id.clone()))
                    .collect()
            })
            .unwrap_or_default();
        symbol_ids.truncate(200);
        if symbol_ids.is_empty() {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "doc_coverage requires at least one `refs[*]` entry with kind=symbol."
                        .to_string(),
                    detail: json!({
                        "analysis": "doc_coverage",
                        "hint": "Pass symbol refs resolved from search_catalog or inspect_structure."
                    }),
                },
            );
        }
        let data = super::quality::doc_coverage::doc_coverage_payload(&doc_store, &symbol_ids)?;
        self.success_response(timer, data)
    }

    fn quality_test_gaps(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let top_n = args.top_n.unwrap_or(25);
        let min_complexity = args.min_complexity.unwrap_or(3);
        let data = codesight_analysis::test_gaps::test_gap_payload(&store, top_n, min_complexity)?;
        self.success_response(&timer, data)
    }
}

fn check_internal_link(
    sec: &codesight_storage::storage::doc_index::SectionRecord,
    target: &str,
    all_files: &HashSet<String>,
    file_headings: &HashMap<String, HashSet<String>>,
) -> Option<Value> {
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
    {
        return None;
    }
    let (file_part, anchor_part) = match target.find('#') {
        Some(pos) => (&target[..pos], Some(&target[pos + 1..])),
        None => (target, None),
    };

    let resolved_file = if file_part.is_empty() {
        sec.file_path.clone()
    } else {
        let base_dir = std::path::Path::new(&sec.file_path)
            .parent()
            .unwrap_or(std::path::Path::new(""));
        base_dir
            .join(file_part)
            .to_string_lossy()
            .replace('\\', "/")
    };

    let reason_detail = if !file_part.is_empty() && !all_files.contains(&resolved_file) {
        Some(("file_not_found", format!("file not found: {resolved_file}")))
    } else if let Some(anchor) = anchor_part {
        let anchor_lower = anchor.to_lowercase();
        let headings_key = if file_part.is_empty() {
            &sec.file_path
        } else {
            &resolved_file
        };
        match file_headings.get(headings_key) {
            Some(headings) if !headings.contains(&anchor_lower) => Some((
                "section_not_found",
                if file_part.is_empty() {
                    format!("anchor '{anchor}' not found")
                } else {
                    format!("anchor '{anchor}' not found in {resolved_file}")
                },
            )),
            _ => None,
        }
    } else {
        None
    };

    reason_detail.map(|(reason_code, detail)| {
        json!({
            "source_file": sec.file_path,
            "source_section": sec.heading,
            "target": target,
            "line": sec.start_line,
            "reason": reason_code,
            "detail": detail,
        })
    })
}
