//! Doc-coverage payload builder.
//!
//! Lives in the root crate because it reads from `DocStore`, which is owned
//! by the root crate's `storage::doc_index` module. Every other quality
//! payload builder lives in `codesight-analysis`; this one stays here until
//! `DocStore` is promoted into `codesight-storage`.

use serde_json::{Value, json};

use codesight_analysis::quality::extract_symbol_name_from_id;

use crate::storage::doc_index::DocStore;

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
    let mut heading_sections = std::collections::HashMap::<String, Vec<Value>>::new();
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
