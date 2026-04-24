//! Fuzzy symbol-name augmentation for `search_catalog`.
//!
//! Two modes (ROADMAP P1.4):
//!   - `auto_fuzzy`: BM25 returned zero, caller supplied a `fuzzy_threshold`.
//!     Fuzzy rows replace the empty exact lane — returned inline via
//!     [`augment_with_fuzzy`], matching today's behaviour.
//!   - `explicit fuzzy=true`: run the fuzzy lane independently and keep it
//!     separate from the exact lane. Produced by [`compute_fuzzy_lane`].
//!     Trigram-Jaccard × Levenshtein; ≥2-edit near-duplicates of an exact
//!     hit are dropped.

use std::collections::HashSet;

use crate::service::agent_v1::args::SearchCatalogArgs;
use codesight_retrieval::fuzzy;
use codesight_storage::storage::{IndexStore, SearchSymbol};

pub(super) fn augment_with_fuzzy(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    auto_fuzzy: bool,
    symbols: &mut Vec<SearchSymbol>,
) -> usize {
    // Auto-fuzzy path: BM25 was empty → fuzzy rows land inline in `symbols`.
    let rows = collect_fuzzy_rows(store, args, scope, exclude, per_kind_cap, auto_fuzzy, symbols);
    let added = rows.len();
    symbols.extend(rows);
    added
}

/// Compute a *separate* fuzzy lane (ROADMAP P1.4). Leaves the exact lane
/// (`primary`) untouched and drops candidates whose edit-distance to any
/// primary hit is ≥ 2 — near-duplicates of an exact match.
pub(super) fn compute_fuzzy_lane(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    primary: &[SearchSymbol],
) -> Vec<SearchSymbol> {
    let mut rows =
        collect_fuzzy_rows(store, args, scope, exclude, per_kind_cap, false, primary);
    // ROADMAP P1.4 §4: if an exact-match hit exists for the same token, drop
    // fuzzy candidates that only differ by ≥2 edits. Reduces near-duplicate
    // noise (`dispatch_tool` vs `dispatch_tools` style pollution).
    rows.retain(|r| {
        primary.iter().all(|p| {
            let diff = fuzzy::score_name_pair(&p.name, &r.name);
            diff.edit_distance >= 2
        })
    });
    rows
}

fn collect_fuzzy_rows(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    auto_fuzzy: bool,
    primary: &[SearchSymbol],
) -> Vec<SearchSymbol> {
    // When the caller opts in with `fuzzy=true`, lower the trigram threshold
    // and enable a default Levenshtein fallback so single-character
    // transpositions in short queries (e.g. `vlaidate` → `validate`) match.
    let threshold = args
        .fuzzy_threshold
        .unwrap_or(if auto_fuzzy { 0.3 } else { 0.25 });
    let max_edit_distance = args
        .max_edit_distance
        .or(if auto_fuzzy { None } else { Some(2) });
    let primary_ids: HashSet<String> = primary.iter().map(|s| s.id.clone()).collect();
    let scan_cap = per_kind_cap.saturating_mul(50).clamp(500, 12_000);
    let Ok(candidates) = store.symbols_light_scan(args.kind.as_deref(), scope, scan_cap) else {
        return Vec::new();
    };
    let mut fuzzy_rows: Vec<SearchSymbol> = Vec::new();
    for mut c in candidates {
        if primary_ids.contains(&c.id) {
            continue;
        }
        if exclude
            .iter()
            .any(|pattern| c.file.starts_with(pattern) || c.file.contains(pattern))
        {
            continue;
        }
        let score = fuzzy::score_name_pair(&args.query, &c.name);
        if fuzzy::fuzzy_candidate_passes(&score, threshold, max_edit_distance) {
            if !auto_fuzzy {
                c.match_type = Some("fuzzy".to_string());
                c.fuzzy_similarity = Some(score.similarity);
                c.edit_distance = Some(score.edit_distance);
            }
            // ROADMAP P1.4 §3: rank fuzzy lane on trigram × Levenshtein, not
            // BM25-style word-match. Small edit distances outrank large ones
            // at equal trigram similarity; an exact Jaccard match with 0
            // edits (e.g. a case-only difference) gets the top slot.
            let edit_penalty = 1.0 / (1.0 + score.edit_distance as f64);
            c.score = score.similarity * edit_penalty;
            fuzzy_rows.push(c);
        }
    }
    fuzzy_rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let take_extra = per_kind_cap.saturating_mul(4).clamp(32, 100);
    fuzzy_rows.truncate(take_extra);
    fuzzy_rows
}
