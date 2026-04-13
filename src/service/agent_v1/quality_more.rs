//! Quality analyses: hotspots, extraction, links, doc coverage.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

static MD_INTERNAL_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Infallible: fixed markdown link pattern.
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").expect("infallible markdown link regex")
});

use crate::analysis::complexity::cyclomatic_complexity;
use crate::analysis::quality;
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_quality_tail(
        &self,
        args: &EvaluateQualityArgs,
        analysis: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        match analysis {
            "hotspots" => self.quality_hotspots(args, timer),
            "extraction_candidates" => self.quality_extraction_candidates(args, timer),
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

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let data = quality::hotspots_payload(&store, min_complexity, top_n)?;

        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [
                ("methodology".to_string(), json!("cyclomatic_plus_churn")),
                ("confidence_level".to_string(), json!("medium")),
            ],
        )
    }

    fn quality_extraction_candidates(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let Some(file_path) = args.file_path.as_deref() else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "extraction_candidates requires file_path.".to_string(),
                    detail: json!({}),
                },
            );
        };
        let min_complexity = args.min_complexity.unwrap_or(0);
        let min_callers = args.min_callers.unwrap_or(0);

        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);

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

        let all_symbols = store.all_symbols_for_analysis()?;
        let mut candidates: Vec<Value> = Vec::new();
        for sym in &all_symbols {
            if sym.file_path != resolved_path {
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
            let (caller_count, caller_files) =
                store.reference_count_and_files(&sym.name, &sym.id)?;
            if (caller_count as u64) < min_callers {
                continue;
            }
            let score = cyclomatic as f64 * (1.0 + caller_count as f64);
            candidates.push(json!({
                "id": sym.id,
                "name": sym.name,
                "kind": sym.kind,
                "file": sym.file_path,
                "line": sym.start_line,
                "cyclomatic": cyclomatic,
                "caller_count": caller_count,
                "caller_files": caller_files,
                "score": score,
            }));
        }

        candidates.sort_by(|a, b| {
            let sa = a["score"].as_f64().unwrap_or(0.0);
            let sb = b["score"].as_f64().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        self.success_response(
            &timer,
            json!({
                "analysis": "extraction_candidates",
                "file": resolved_path,
                "candidates": candidates,
            }),
        )
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
        let data = quality::doc_coverage_payload(&doc_store, &symbol_ids)?;
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
        let data = crate::analysis::test_gaps::test_gap_payload(&store, top_n, min_complexity)?;
        self.success_response(&timer, data)
    }
}

fn check_internal_link(
    sec: &crate::storage::doc_index::SectionRecord,
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
