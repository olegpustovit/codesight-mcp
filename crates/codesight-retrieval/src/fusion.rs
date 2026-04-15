//! Reciprocal Rank Fusion over a lexical + dense candidate pair.
//!
//! Minimal implementation — plain RRF with the canonical k=60 constant.
//! No weighted-sum mode, no score normalization (§8.2 of
//! `IDF_REPLACEMENT.md`).

use std::collections::HashMap;

const RRF_K: f64 = 60.0;

/// Combine two ranked id lists (each already sorted by relevance) into a
/// single ranked list by summing reciprocal ranks. Stable for ties.
pub fn rrf<I, J>(lexical: I, dense: J) -> Vec<(String, f64)>
where
    I: IntoIterator<Item = String>,
    J: IntoIterator<Item = String>,
{
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, id) in lexical.into_iter().enumerate() {
        *scores.entry(id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    for (rank, id) in dense.into_iter().enumerate() {
        *scores.entry(id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    let mut out: Vec<(String, f64)> = scores.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuses_disjoint_lists_preserving_both() {
        let lex = vec!["a".to_string(), "b".to_string()];
        let dense = vec!["c".to_string(), "d".to_string()];
        let fused = rrf(lex, dense);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c", "b", "d"]);
    }

    #[test]
    fn boosts_items_found_in_both_lanes() {
        let lex = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let dense = vec!["c".to_string(), "x".to_string()];
        let fused = rrf(lex, dense);
        // c appears in both lanes → its reciprocal ranks (1/63 + 1/61) sum above
        // any single-lane item's score; it must win.
        assert_eq!(fused[0].0, "c", "both-lane item must lead");
        let pos = |id: &str| fused.iter().position(|(s, _)| s == id).unwrap();
        assert!(pos("c") < pos("a"), "c outranks single-lane a");
        assert!(pos("a") < pos("b"), "higher-ranked lex item outranks lower");
    }
}
