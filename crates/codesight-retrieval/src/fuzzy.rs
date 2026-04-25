//! Fuzzy symbol-name matching backed by `nucleo-matcher` (Helix editor's
//! fuzzy lib). Replaces the prior trigram + Damerau-Levenshtein gate,
//! which mis-ranked typos like `SavingLedger` → `savings_file`.
//!
//! Also hosts `tokenize_name` / `relatedness_score`, which the `related`
//! graph mode needs and which are unrelated to name-vs-query fuzzy ranking.

use rustc_hash::FxHashSet;

use heck::ToSnakeCase;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use strsim::jaro_winkler;

/// Case-folded Jaro-Winkler similarity (0.0..=1.0). Use for the *honest*
/// "how close is this to a typo" signal — `disptchTool` vs `dispatch_tool`
/// scores ~0.92 here even though the nucleo subsequence score for the same
/// pair sits at 1.0 (every char of the query appears in subsequence order
/// in the candidate). The two metrics answer different questions; keep both.
pub fn edit_similarity(query: &str, candidate: &str) -> f64 {
    let q = query.to_ascii_lowercase();
    let c = candidate.to_ascii_lowercase();
    jaro_winkler(&q, &c).clamp(0.0, 1.0)
}

/// Score a single candidate against a query. Returns `None` when the query
/// isn't a fuzzy subsequence of the candidate. When it matches, the score
/// is normalized to `[0.0, 1.0]`: 1.0 is the best possible raw score for
/// this `(query, candidate)` length pair.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<f32> {
    let mut matcher = new_matcher();
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let raw = pattern.score(Utf32Str::new(candidate, &mut buf), &mut matcher)?;
    // Normalize against a self-score so "query == candidate" -> 1.0.
    let mut self_buf = Vec::new();
    let best = pattern
        .score(Utf32Str::new(query, &mut self_buf), &mut matcher)
        .unwrap_or(raw)
        .max(1);
    Some((raw as f32 / best as f32).clamp(0.0, 1.0))
}

/// Candidate input for [`fuzzy_rank`]. `kind_priority` is a caller-supplied
/// tie-breaker (higher wins) used only when two candidates produce identical
/// nucleo scores. Keep it small: the caller owns the mapping table.
pub struct FuzzyCandidate<'a> {
    pub name: &'a str,
    pub kind_priority: u8,
}

impl<'a> From<&'a str> for FuzzyCandidate<'a> {
    fn from(name: &'a str) -> Self {
        Self {
            name,
            kind_priority: 0,
        }
    }
}

/// Rank a batch of candidates. Returns `(index, normalized_score)` pairs,
/// sorted by score descending. Ties on score break on shorter name first
/// (intent: typo closer to shorter symbol), then on higher `kind_priority`,
/// then on original index ascending. Normalized against the best raw score
/// in the batch. Candidates scoring below 20% of the best are dropped so
/// long-range subsequence matches don't pollute the lane.
pub fn fuzzy_rank<'a, I>(query: &str, candidates: I) -> Vec<(usize, f32)>
where
    I: IntoIterator,
    I::Item: Into<FuzzyCandidate<'a>>,
{
    const FLOOR_FRAC: f32 = 0.2;
    let mut matcher = new_matcher();
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    // Carry name length + kind priority through the sort without re-reading
    // candidates after the nucleo pass.
    let mut raw: Vec<(usize, u32, usize, u8)> = Vec::new();
    let mut buf: Vec<char> = Vec::new();
    for (i, cand) in candidates.into_iter().enumerate() {
        let c: FuzzyCandidate<'a> = cand.into();
        if let Some(score) = pattern.score(Utf32Str::new(c.name, &mut buf), &mut matcher) {
            raw.push((i, score, c.name.len(), c.kind_priority));
        }
    }
    let Some(&(_, best, _, _)) = raw.iter().max_by_key(|(_, s, _, _)| *s) else {
        return Vec::new();
    };
    let best_f = best.max(1) as f32;
    let floor = best_f * FLOOR_FRAC;
    let mut kept: Vec<(usize, u32, usize, u8)> = raw
        .into_iter()
        .filter(|(_, s, _, _)| (*s as f32) >= floor)
        .collect();
    kept.sort_by(|a, b| {
        // Primary: higher raw nucleo score wins.
        b.1.cmp(&a.1)
            // Tie #1: shorter candidate name wins.
            .then_with(|| a.2.cmp(&b.2))
            // Tie #2: higher kind_priority wins.
            .then_with(|| b.3.cmp(&a.3))
            // Final: stable by original index.
            .then_with(|| a.0.cmp(&b.0))
    });
    kept.into_iter()
        .map(|(i, s, _, _)| (i, (s as f32 / best_f).clamp(0.0, 1.0)))
        .collect()
}

fn new_matcher() -> Matcher {
    Matcher::new(Config::DEFAULT)
}

/// Tokenize a symbol name for relatedness scoring (underscore + camel segments).
pub fn tokenize_name(name: &str) -> FxHashSet<String> {
    // Convert CamelCase to snake_case first, then split on underscores.
    // `heck::ToSnakeCase` handles acronyms and mixed cases correctly.
    name.to_snake_case()
        .split('_')
        .filter(|s| s.len() >= 2)
        .map(str::to_string)
        .collect()
}

/// Relatedness score for the `related` graph mode — token-set Jaccard with
/// a same-file bonus. Independent of the nucleo-backed fuzzy ranker.
pub fn relatedness_score(
    source_name: &str,
    source_file: &str,
    other_name: &str,
    other_file: &str,
) -> f64 {
    let same_file = source_file == other_file;
    let a = tokenize_name(source_name);
    let b = tokenize_name(other_name);
    if a.is_empty() && b.is_empty() {
        return if same_file { 0.5 } else { 0.0 };
    }
    let inter = a.intersection(&b).count() as f64;
    let union = a.union(&b).count().max(1) as f64;
    let j = inter / union;
    if same_file { 0.5 + j * 0.75 } else { j }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typo_ranks_nearest_name_first() {
        // The regression that motivated the nucleo swap.
        let names = ["savings_file", "savings_entry", "SavingsLedger", "unrelated"];
        let ranked = fuzzy_rank("SavingLedger", names.iter().copied());
        let first = ranked.first().expect("at least one match");
        assert_eq!(names[first.0], "SavingsLedger");
    }

    #[test]
    fn tie_break_shorter_name_wins() {
        // Both candidates are exact subsequence matches of the query and
        // score identically. The shorter name should rank first — a
        // 13-char typo is closer to `SavingsLedger` than to a 40-char
        // method sharing the same subsequence.
        let names = [
            "savings_ledger_set_quality_session_start",
            "SavingsLedger",
        ];
        let ranked = fuzzy_rank("SavingLedger", names.iter().copied());
        assert!(ranked.len() >= 2, "expected both to rank, got {ranked:?}");
        // Equal scores → shorter wins.
        assert!((ranked[0].1 - ranked[1].1).abs() < f32::EPSILON);
        assert_eq!(names[ranked[0].0], "SavingsLedger");
    }

    #[test]
    fn tie_break_kind_priority_wins_on_equal_length() {
        // Equal query match and equal name length → higher kind_priority wins.
        let cands = [
            FuzzyCandidate { name: "aB", kind_priority: 1 },
            FuzzyCandidate { name: "aC", kind_priority: 9 },
        ];
        let ranked = fuzzy_rank("a", cands);
        assert!(ranked.len() >= 2, "{ranked:?}");
        assert_eq!(ranked[0].0, 1, "expected higher kind_priority first; {ranked:?}");
    }

    #[test]
    fn exact_match_scores_one() {
        assert_eq!(fuzzy_score("foo_bar", "foo_bar"), Some(1.0));
    }

    #[test]
    fn non_match_returns_none() {
        assert!(fuzzy_score("xyz", "abc").is_none());
    }
}
