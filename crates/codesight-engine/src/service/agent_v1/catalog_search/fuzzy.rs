//! Fuzzy symbol-name augmentation for `search_catalog`.
//!
//! Backed by `nucleo-matcher` via `codesight_retrieval::fuzzy::fuzzy_rank`.
//! Two modes:
//!   - `auto_fuzzy`: BM25 returned zero hits → fuzzy rows replace the empty
//!     exact lane ([`augment_with_fuzzy`]).
//!   - `explicit fuzzy=true`: run the fuzzy lane alongside the exact lane
//!     ([`compute_fuzzy_lane`]).

use std::collections::HashSet;

use crate::service::agent_v1::args::SearchCatalogArgs;
use codesight_core::SymbolKind;
use codesight_retrieval::fuzzy::{self, FuzzyCandidate};
use codesight_storage::storage::{IndexStore, SearchSymbol};

/// Tie-break priority for the fuzzy lane. Only applied when two candidates
/// produce identical nucleo scores — a higher priority wins. Rationale: for
/// a typo'd identifier query the user almost certainly wants the type
/// definition over a method that happens to share the same subsequence.
fn kind_priority(kind: SymbolKind) -> u8 {
    match kind {
        // Types / definitions — what the user is most often after.
        SymbolKind::Class | SymbolKind::Type => 5,
        // Callables come next.
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Template => 3,
        // Constants / values.
        SymbolKind::Constant => 2,
        // Re-export trail — almost never the thing a fuzzy query wants.
        SymbolKind::Import | SymbolKind::Reexport => 1,
    }
}

pub(super) fn augment_with_fuzzy(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    auto_fuzzy: bool,
    symbols: &mut Vec<SearchSymbol>,
) -> usize {
    let rows = collect_fuzzy_rows(store, args, scope, exclude, per_kind_cap, auto_fuzzy, symbols);
    let added = rows.len();
    symbols.extend(rows);
    added
}

/// Compute a *separate* fuzzy lane, leaving the exact lane (`primary`)
/// untouched.
pub(super) fn compute_fuzzy_lane(
    store: &IndexStore,
    args: &SearchCatalogArgs,
    scope: Option<&str>,
    exclude: &[String],
    per_kind_cap: usize,
    primary: &[SearchSymbol],
) -> Vec<SearchSymbol> {
    collect_fuzzy_rows(store, args, scope, exclude, per_kind_cap, false, primary)
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
    let primary_ids: HashSet<String> = primary.iter().map(|s| s.id.clone()).collect();
    // The fuzzy lane must see every symbol in the index (post `kind` / `scope`
    // filtering) — anything less means typos pointing to late-sorting files
    // silently miss. `symbols_light_scan` clamps internally to 20_000, which
    // covers every repo we ship for; extremely large monorepos degrade to
    // "top N by file_path" instead of "nothing at all".
    let Ok(mut candidates) = store.symbols_light_scan(args.kind.as_deref(), scope, 20_000) else {
        return Vec::new();
    };

    // Prune exact duplicates + vendored exclusions before ranking so
    // nucleo doesn't waste cycles on rows we'd drop.
    candidates.retain(|c| {
        if primary_ids.contains(&c.id) {
            return false;
        }
        !exclude
            .iter()
            .any(|pattern| c.file.starts_with(pattern) || c.file.contains(pattern))
    });

    let fuzzy_inputs: Vec<FuzzyCandidate<'_>> = candidates
        .iter()
        .map(|c| FuzzyCandidate {
            name: c.name.as_str(),
            kind_priority: kind_priority(c.kind),
        })
        .collect();
    let ranked = fuzzy::fuzzy_rank(&args.query, fuzzy_inputs);

    let take_extra = per_kind_cap.saturating_mul(4).clamp(32, 100);
    let mut out: Vec<SearchSymbol> = Vec::with_capacity(ranked.len().min(take_extra));
    for (idx, score) in ranked.into_iter().take(take_extra) {
        let mut row = candidates[idx].clone();
        if !auto_fuzzy {
            row.match_type = Some("fuzzy".to_string());
            row.fuzzy_similarity = Some(score as f64);
            row.edit_distance = None;
        }
        row.score = score as f64;
        out.push(row);
    }
    out
}
