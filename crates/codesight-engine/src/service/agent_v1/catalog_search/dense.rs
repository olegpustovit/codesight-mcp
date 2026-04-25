//! Dense-lane helpers: similarity conversion, row materialization, and RRF
//! fusion of lexical + dense results.

use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::service::CodeService;
use crate::service::agent_v1::util::{search_hit_section, search_hit_symbol};
use crate::service::helpers::{ToolError, index_error};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

use super::semantic::DenseRankedHit;

pub(super) fn dense_similarity(distance: f64) -> f64 {
    1.0 / (1.0 + distance.max(0.0))
}

pub(super) fn materialize_dense_symbol_items(
    svc: &CodeService,
    source_id: &str,
    store: Option<&IndexStore>,
    dense_hits: &[DenseRankedHit],
    include_body: bool,
    context_lines: usize,
) -> Result<FxHashMap<String, Value>, ToolError> {
    let Some(store) = store else {
        return Ok(FxHashMap::default());
    };
    let ids: Vec<String> = dense_hits.iter().map(|hit| hit.id.clone()).collect();
    let scores: FxHashMap<&str, f64> = dense_hits
        .iter()
        .map(|hit| (hit.id.as_str(), dense_similarity(hit.distance)))
        .collect();
    let rows = store.search_symbols_by_ids(&ids).map_err(index_error)?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.score = *scores.get(row.id.as_str()).unwrap_or(&0.0);
            let id = row.id.clone();
            let body = if include_body {
                svc.symbol_body_with_context(store, &row.id, context_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            (id, search_hit_symbol(source_id, &row, body.as_deref()))
        })
        .collect())
}

pub(super) fn materialize_dense_section_items(
    svc: &CodeService,
    source_id: &str,
    store: Option<&DocStore>,
    dense_hits: &[DenseRankedHit],
    include_body: bool,
    context_lines: usize,
) -> Result<FxHashMap<String, Value>, ToolError> {
    let Some(store) = store else {
        return Ok(FxHashMap::default());
    };
    let ids: Vec<String> = dense_hits.iter().map(|hit| hit.id.clone()).collect();
    let scores: FxHashMap<&str, f64> = dense_hits
        .iter()
        .map(|hit| (hit.id.as_str(), dense_similarity(hit.distance)))
        .collect();
    let rows = store.search_sections_by_ids(&ids).map_err(index_error)?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.score = *scores.get(row.id.as_str()).unwrap_or(&0.0);
            let id = row.id.clone();
            let body = if include_body {
                svc.section_body_with_context(store, &row.id, context_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            (id, search_hit_section(source_id, &row, body.as_deref()))
        })
        .collect())
}

/// Fuse the already-ranked lexical items with the dense lane's id lists
/// via RRF. Symbol and section lanes fuse separately; file-text hits
/// (which the dense lane never produces) pass through unchanged.
pub(super) fn merge_dense_ranked_items(
    items: Vec<Value>,
    dense_symbol_hits: &[DenseRankedHit],
    dense_section_hits: &[DenseRankedHit],
    mut dense_symbol_items: FxHashMap<String, Value>,
    mut dense_section_items: FxHashMap<String, Value>,
) -> Vec<Value> {
    let mut lex_symbols: Vec<String> = Vec::new();
    let mut lex_sections: Vec<String> = Vec::new();
    let mut symbol_items: FxHashMap<String, Value> =
        FxHashMap::default();
    let mut section_items: FxHashMap<String, Value> =
        FxHashMap::default();
    let mut others: Vec<Value> = Vec::new();
    for item in items {
        let id = item
            .get("ref")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let kind = item
            .get("ref")
            .and_then(|r| r.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match (id, kind) {
            (Some(i), "symbol") => {
                lex_symbols.push(i.clone());
                symbol_items.insert(i, item);
            }
            (Some(i), "section") => {
                lex_sections.push(i.clone());
                section_items.insert(i, item);
            }
            _ => others.push(item),
        }
    }

    let fused_symbols = codesight_retrieval::fusion::rrf(
        lex_symbols,
        dense_symbol_hits.iter().map(|hit| hit.id.clone()),
    );
    let fused_sections = codesight_retrieval::fusion::rrf(
        lex_sections,
        dense_section_hits.iter().map(|hit| hit.id.clone()),
    );

    let mut out: Vec<Value> = Vec::with_capacity(
        others.len()
            + symbol_items.len()
            + section_items.len()
            + dense_symbol_items.len()
            + dense_section_items.len(),
    );
    for (id, _score) in fused_symbols {
        if let Some(item) = symbol_items.remove(&id) {
            out.push(item);
        } else if let Some(item) = dense_symbol_items.remove(&id) {
            out.push(item);
        }
    }
    for (id, _score) in fused_sections {
        if let Some(item) = section_items.remove(&id) {
            out.push(item);
        } else if let Some(item) = dense_section_items.remove(&id) {
            out.push(item);
        }
    }
    out.extend(others);
    out
}
