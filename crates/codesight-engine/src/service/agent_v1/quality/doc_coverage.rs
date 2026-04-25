//! `evaluate_quality analysis=doc_coverage` — match symbols to documentation sections.

use rustc_hash::FxHashMap;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use codesight_analysis::quality::extract_symbol_name_from_id;
use codesight_storage::storage::doc_index::DocStore;

impl CodeService {
    pub(super) fn quality_doc_coverage(
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
        let data = doc_coverage_payload(&doc_store, &symbol_ids)?;
        let _ = timer;
        // Heading-name match is a heuristic alignment between symbol names
        // and section headings — partial.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);
        self.success_response(&timer, data)
    }
}

pub fn doc_coverage_payload(doc_store: &DocStore, symbol_ids: &[String]) -> anyhow::Result<Value> {
    if symbol_ids.is_empty() {
        return Ok(json!({
            "analysis": "doc_coverage",
            "symbols_checked": 0,
            "documented_count": 0,
            "undocumented_count": 0,
            "coverage_pct": 0.0,
            "documented": [],
            "undocumented": [],
        }));
    }

    let all_sections = doc_store.all_sections()?;
    let mut heading_sections = FxHashMap::<String, Vec<Value>>::default();
    for section in &all_sections {
        heading_sections
            .entry(section.heading.to_lowercase())
            .or_default()
            .push(json!({
                "section_id": section.id,
                "section_title": section.heading,
                "doc_path": section.file_path,
            }));
    }

    let mut documented = Vec::new();
    let mut undocumented = Vec::new();
    for symbol_id in symbol_ids {
        let symbol_name = extract_symbol_name_from_id(symbol_id);
        let heading_key = symbol_name.to_lowercase();
        if let Some(sections) = heading_sections.get(&heading_key) {
            documented.push(json!({
                "symbol_id": symbol_id,
                "symbol_name": symbol_name,
                "matching_sections": sections,
            }));
        } else {
            undocumented.push(json!({
                "symbol_id": symbol_id,
                "symbol_name": symbol_name,
            }));
        }
    }

    let documented_count = documented.len();
    let total = symbol_ids.len();
    let coverage_pct = if total == 0 {
        0.0
    } else {
        documented_count as f64 / total as f64 * 100.0
    };

    let summary = doc_store.doc_summary()?;

    Ok(json!({
        "analysis": "doc_coverage",
        "repo": summary["source_id"],
        "symbols_checked": total,
        "documented_count": documented_count,
        "undocumented_count": undocumented.len(),
        "coverage_pct": coverage_pct,
        "documented": documented,
        "undocumented": undocumented,
    }))
}
