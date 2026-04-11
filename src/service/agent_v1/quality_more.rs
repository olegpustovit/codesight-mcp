//! Quality analyses: hotspots, extraction, links, doc coverage.

use std::collections::HashSet;
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
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_quality_tail(
        &self,
        args: &EvaluateQualityArgs,
        analysis: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        match analysis {
            "hotspots" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(timer, e),
                };
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
            "extraction_candidates" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(timer, e),
                };
                let file_path = match args.file_path.as_deref() {
                    Some(fp) => fp.to_string(),
                    None => {
                        return self.error_response(
                            timer,
                            ToolError {
                                category: ToolErrorCategory::InvalidArgument,
                                code: "INVALID_REQUEST",
                                message: "extraction_candidates requires file_path.".to_string(),
                                detail: json!({}),
                            },
                        );
                    }
                };
                let min_complexity = args.min_complexity.unwrap_or(0);
                let min_callers = args.min_callers.unwrap_or(0);

                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);

                // Check file exists
                // Try exact match first, then try finding the file with flexible matching
                let all_files = store.all_file_paths()?;
                let resolved_path = all_files.iter().find(|f| {
                    *f == &file_path
                        || f.ends_with(&format!("/{file_path}"))
                        || f.ends_with(&file_path)
                });
                let resolved_path = match resolved_path {
                    Some(p) => p.clone(),
                    None => {
                        return self.error_response(
                            &timer,
                            ToolError {
                                category: ToolErrorCategory::NotFound,
                                code: "NOT_FOUND",
                                message: format!("File '{file_path}' not found in index."),
                                detail: json!({ "file_path": file_path }),
                            },
                        );
                    }
                };

                let all_symbols = store.all_symbols_for_analysis()?;

                let mut candidates: Vec<Value> = Vec::new();
                for sym in &all_symbols {
                    if sym.file_path != resolved_path {
                        continue;
                    }
                    // Only functions/methods
                    let kind_lower = sym.kind.to_ascii_lowercase();
                    if kind_lower != "function" && kind_lower != "method" {
                        continue;
                    }

                    let cyclomatic = cyclomatic_complexity(&sym.source_text) as u64;

                    if cyclomatic < min_complexity {
                        continue;
                    }

                    // Caller info
                    let (caller_count, caller_files) =
                        store.reference_count_and_files(&sym.name, &sym.id)?;
                    if (caller_count as u64) < min_callers {
                        continue;
                    }

                    // Score: cyclomatic * (1 + caller_count)
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

                // Sort by score descending
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
            "broken_links" => {
                let doc_store =
                    match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                        Ok(s) => s,
                        Err(e) => return self.error_response(timer, e),
                    };
                let all_sections = doc_store.all_sections()?;
                let all_files: HashSet<String> = doc_store.all_doc_files()?.into_iter().collect();

                // Build a map of file -> headings for anchor validation
                let mut file_headings: std::collections::HashMap<String, HashSet<String>> =
                    std::collections::HashMap::new();
                for sec in &all_sections {
                    file_headings
                        .entry(sec.file_path.clone())
                        .or_default()
                        .insert(sec.heading.to_lowercase().replace(' ', "-"));
                }

                let mut broken_links: Vec<Value> = Vec::new();

                // Scan each doc file for internal links
                for sec in &all_sections {
                    let content = &sec.content_text;
                    // Find markdown links: [text](target)
                    for cap in MD_INTERNAL_LINK_RE.captures_iter(content) {
                        let Some(target) = cap.get(2).map(|m| m.as_str()) else {
                            continue;
                        };
                        // Skip external links
                        if target.starts_with("http://")
                            || target.starts_with("https://")
                            || target.starts_with("mailto:")
                        {
                            continue;
                        }
                        // Parse file and anchor
                        let (file_part, anchor_part) = if let Some(pos) = target.find('#') {
                            (&target[..pos], Some(&target[pos + 1..]))
                        } else {
                            (target, None)
                        };

                        let resolved_file = if file_part.is_empty() {
                            sec.file_path.clone()
                        } else {
                            // Resolve relative path
                            let base_dir = std::path::Path::new(&sec.file_path)
                                .parent()
                                .unwrap_or(std::path::Path::new(""));
                            let resolved = base_dir.join(file_part);
                            resolved.to_string_lossy().replace('\\', "/")
                        };

                        let mut is_broken = false;
                        let mut broken_reason = String::new();

                        if !file_part.is_empty() && !all_files.contains(&resolved_file) {
                            is_broken = true;
                            broken_reason = format!("file not found: {}", resolved_file);
                        } else if let Some(anchor) = anchor_part {
                            let anchor_lower = anchor.to_lowercase();
                            let headings = file_headings.get(&resolved_file);
                            if let Some(headings) = headings {
                                if !headings.contains(&anchor_lower) {
                                    is_broken = true;
                                    broken_reason = format!(
                                        "anchor '{}' not found in {}",
                                        anchor, resolved_file
                                    );
                                }
                            } else if file_part.is_empty() {
                                // Anchor-only link in current file
                                let headings = file_headings.get(&sec.file_path);
                                if let Some(headings) = headings
                                    && !headings.contains(&anchor_lower)
                                {
                                    is_broken = true;
                                    broken_reason = format!("anchor '{}' not found", anchor);
                                }
                            }
                        }

                        if is_broken {
                            let reason_code = if broken_reason.starts_with("file not found") {
                                "file_not_found"
                            } else {
                                "section_not_found"
                            };
                            broken_links.push(json!({
                                "source_file": sec.file_path,
                                "source_section": sec.heading,
                                "target": target,
                                "line": sec.start_line,
                                "reason": reason_code,
                                "detail": broken_reason,
                            }));
                        }
                    }
                }

                let docs_scanned = all_files.len();
                let sections_scanned = all_sections.len();

                self.success_response(
                    timer,
                    json!({
                        "analysis": "broken_links",
                        "repo": args.source_id,
                        "docs_scanned": docs_scanned,
                        "sections_scanned": sections_scanned,
                        "broken_link_count": broken_links.len(),
                        "broken_links": broken_links,
                    }),
                )
            }
            "doc_coverage" => {
                // doc_coverage works with the doc store (not code store)
                let doc_store =
                    match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                        Ok(s) => s,
                        Err(e) => return self.error_response(timer, e),
                    };

                // Get symbol IDs from refs (cap at 200)
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
}
