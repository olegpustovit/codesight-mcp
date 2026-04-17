//! Fuzzy symbol-name augmentation for `search_catalog`.
//!
//! Adds near-matches below the BM25 threshold, gated on an explicit `fuzzy=true`
//! flag or on zero BM25 hits when a `fuzzy_threshold` is supplied.

use std::collections::HashSet;

use crate::service::agent_v1::args::SearchCatalogArgs;
use codesight_storage::storage::{IndexStore, SearchSymbol};
use codesight_retrieval::fuzzy;

pub(super) fn augment_with_fuzzy(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    auto_fuzzy: bool,
    symbols: &mut Vec<SearchSymbol>,
) -> usize {
    // When the caller opts in with `fuzzy=true`, lower the trigram threshold
    // and enable a default Levenshtein fallback so single-character
    // transpositions in short queries (e.g. `vlaidate` → `validate`) match.
    // Trigram Jaccard alone degrades quickly on short queries: swapping two
    // adjacent letters drops the score below 0.3. An edit-distance bound of 2
    // catches these without polluting longer queries, because the bound is
    // absolute characters and `normalize_fuzzy_compare` strips underscores.
    let threshold = args
        .fuzzy_threshold
        .unwrap_or(if auto_fuzzy { 0.3 } else { 0.25 });
    let max_edit_distance = args
        .max_edit_distance
        .or(if auto_fuzzy { None } else { Some(2) });
    let primary_ids: HashSet<String> = symbols.iter().map(|s| s.id.clone()).collect();
    let scan_cap = per_kind_cap.saturating_mul(50).clamp(500, 12_000);
    let Ok(candidates) = store.symbols_light_scan(args.kind.as_deref(), scope, scan_cap) else {
        return 0;
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
            c.score = score.similarity * 0.45;
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
    let added = fuzzy_rows.len();
    symbols.extend(fuzzy_rows);
    added
}
