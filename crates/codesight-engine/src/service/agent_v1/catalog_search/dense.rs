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
    let mut symbol_items: FxHashMap<String, Value> = FxHashMap::default();
    let mut section_items: FxHashMap<String, Value> = FxHashMap::default();
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

/// Tier 7.1: rule-based post-RRF rerank for natural-language search.
///
/// RRF is rank-only — it discards score information, so a
/// setter / getter / record_* helper that ranks well in BM25 *and* dense
/// can outrank the actual answer the caller is asking about. This pass
/// applies three deterministic adjustments inspired by Sourcegraph's
/// BM25F field weighting and standard hybrid-search post-rerank:
///
/// - **+0.15** when the item's symbol name (the last segment) matches one
///   of the query tokens — Sourcegraph found symbol-definition matches
///   are ~20% more meaningful than body matches.
/// - **−0.10** for setter/getter shape names (`set_*`, `get_*`,
///   `record_*`, `with_*`, `*_set_*`, `*_record_*`) when a non-prefixed
///   candidate (the bare root) also appears in this response — those
///   helpers are usually noise relative to the canonical accessor.
/// - **−0.05** for test/internal symbols (path contains `tests::`, name
///   ends in `_test`) when the query doesn't mention "test".
///
/// **Caveat (acknowledged):** the proper hybrid-search lift is a
/// cross-encoder reranker on the top-50 candidates. This rule-based pass
/// is the deterministic stopgap until the cross-encoder slot is wired
/// (see worklog: "Tier 7 follow-up — cross-encoder reranker"). Callers
/// who care about strict ranking quality should still verify against
/// `confidence: "partial"` and the warning codes.
pub(super) fn apply_nl_rerank(items: &mut Vec<Value>, query: &str) {
    if items.len() < 2 {
        return;
    }
    let query_lower = query.to_ascii_lowercase();
    let query_tokens: Vec<String> = query_lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    let query_mentions_test = query_lower.contains("test");

    // Collect bare roots: if `record_x` exists alongside `x`, keep the bare
    // `x` as a "non-prefixed candidate" reference for the setter penalty.
    let bare_roots: rustc_hash::FxHashSet<String> = items
        .iter()
        .filter_map(|item| name_of(item).map(|n| n.to_ascii_lowercase()))
        .filter(|n| {
            !n.starts_with("set_")
                && !n.starts_with("get_")
                && !n.starts_with("record_")
                && !n.starts_with("with_")
        })
        .collect();

    let scores: Vec<f64> = items
        .iter()
        .map(|item| score_one(item, &query_tokens, query_mentions_test, &bare_roots))
        .collect();

    // Stable-sort: ties preserve RRF order. Only re-sort when any score
    // is non-zero — pure rank passthrough otherwise.
    if scores.iter().all(|s| s.abs() < f64::EPSILON) {
        return;
    }
    let mut indexed: Vec<(usize, f64)> = scores.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let reordered: Vec<Value> = indexed
        .into_iter()
        .map(|(idx, _)| std::mem::take(&mut items[idx]))
        .collect();
    *items = reordered;
}

fn name_of(item: &Value) -> Option<&str> {
    item.pointer("/ref/title")
        .and_then(Value::as_str)
        .or_else(|| item.get("name").and_then(Value::as_str))
        .or_else(|| {
            item.pointer("/ref/id")
                .and_then(Value::as_str)
                .map(|id| id.rsplit_once("::").map(|(_, t)| t).unwrap_or(id))
                .map(|t| t.split('#').next().unwrap_or(t))
                .map(|t| t.rsplit_once('.').map(|(_, n)| n).unwrap_or(t))
        })
}

fn score_one(
    item: &Value,
    query_tokens: &[String],
    query_mentions_test: bool,
    bare_roots: &rustc_hash::FxHashSet<String>,
) -> f64 {
    let mut delta = 0.0_f64;
    let Some(name) = name_of(item) else {
        return 0.0;
    };
    let lower = name.to_ascii_lowercase();

    // (a) Symbol-name token match.
    if query_tokens.iter().any(|t| t == &lower) {
        delta += 0.15;
    }

    // (b) Setter/getter/record/with penalty when a bare root exists.
    let prefixes: &[&str] = &["set_", "get_", "record_", "with_"];
    for p in prefixes {
        if let Some(stripped) = lower.strip_prefix(p)
            && bare_roots.contains(stripped)
        {
            delta -= 0.10;
            break;
        }
    }

    // (c) Test-symbol penalty.
    if !query_mentions_test {
        let id = item
            .pointer("/ref/id")
            .and_then(Value::as_str)
            .map(|s| s.to_ascii_lowercase());
        let in_tests = id
            .as_deref()
            .map(|s| s.contains("tests::") || s.contains(".tests.") || s.contains("/tests/"))
            .unwrap_or(false);
        let test_suffix = lower.ends_with("_test") || lower.ends_with("_tests");
        if in_tests || test_suffix {
            delta -= 0.05;
        }
    }

    delta
}
