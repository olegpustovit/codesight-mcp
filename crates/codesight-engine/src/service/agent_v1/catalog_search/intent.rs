//! Query-intent classifier for `search_catalog`.
//!
//! Decides whether a natural-language query is asking about *code* (penalize
//! doc sections so structural answers win), *docs* (slight boost to doc
//! sections), or is *unknown* (no change). The classifier is heuristic and
//! intentionally simple — it exists to break ties on hybrid queries where
//! prose-y doc-section hits would otherwise outrank the actual code symbol
//! within ~0.005 of score.

use serde_json::Value;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum QueryIntent {
    Code,
    Doc,
    Unknown,
}

impl QueryIntent {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            QueryIntent::Code => "code",
            QueryIntent::Doc => "doc",
            QueryIntent::Unknown => "unknown",
        }
    }

    /// Base multiplier applied to doc-section score when this intent is in
    /// effect. Stacks multiplicatively with the vendor penalty. The Code
    /// path may soften to `0.5` when the best raw doc and code scores are
    /// already close — see [`effective_doc_section_multiplier`].
    pub(super) fn doc_section_multiplier(self) -> f64 {
        match self {
            QueryIntent::Code => 0.4,
            QueryIntent::Doc => 1.15,
            QueryIntent::Unknown => 1.0,
        }
    }
}

/// Maximum gap (best_doc - best_code) at which the Code-intent multiplier
/// softens from `0.4` to `0.5`. When doc and code answers are already
/// within this gap, the score field doesn't need a hard demotion to break
/// the tie.
const CLOSE_GAP_THRESHOLD: f64 = 0.3;
const CLOSE_GAP_MULTIPLIER: f64 = 0.5;

/// Compute the effective doc-section multiplier given the current items.
/// For `Code` intent, softens to `0.5` when the best raw-score code item
/// and best raw-score doc-section item are within `CLOSE_GAP_THRESHOLD`
/// (in either direction). For other intents, returns the base multiplier.
pub(super) fn effective_doc_section_multiplier(intent: QueryIntent, items: &[Value]) -> f64 {
    let base = intent.doc_section_multiplier();
    if intent != QueryIntent::Code {
        return base;
    }
    let mut best_doc: Option<f64> = None;
    let mut best_code: Option<f64> = None;
    for item in items {
        let s = item.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        let slot = if is_doc_section_item(item) { &mut best_doc } else { &mut best_code };
        if slot.is_none_or(|cur| s > cur) {
            *slot = Some(s);
        }
    }
    match (best_code, best_doc) {
        (Some(c), Some(d)) if (d - c).abs() <= CLOSE_GAP_THRESHOLD => CLOSE_GAP_MULTIPLIER,
        _ => base,
    }
}

/// Phrases that signal the user is asking *about code* — implementations,
/// call sites, behavior. Matched case-insensitive against the trimmed query.
const CODE_PHRASES: &[&str] = &[
    "how does ",
    "how do ",
    "how is ",
    "how are ",
    "where is ",
    "where does ",
    "where are ",
    "implementation of ",
    "function that ",
    "method that ",
    "fn that ",
    "calls ",
    "returns ",
    "called by ",
    "uses ",
    "usage of ",
    "who calls ",
];

/// Phrases that signal the user is asking *about documentation* — overviews,
/// guides, conceptual explanations. Matched case-insensitive against the
/// trimmed query.
const DOC_PHRASES: &[&str] = &[
    "what is ",
    "what are ",
    "explain ",
    "guide for ",
    "guide to ",
    "documentation on ",
    "documentation for ",
    "docs for ",
    "docs on ",
    "overview of ",
    "tutorial ",
];

/// True when the trimmed query is a single identifier-like token: snake_case,
/// CamelCase, kebab-case, or `Type::method` / `pkg.Type` shapes. These
/// almost always mean "find this symbol", not "tell me about this concept".
fn looks_like_identifier(trimmed: &str) -> bool {
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    // At least one ascii letter, only identifier-shaped chars.
    let mut has_letter = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphabetic() {
            has_letter = true;
        } else if !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')) {
            return false;
        }
    }
    has_letter
}

/// Classify a free-text search query into a coarse intent. Heuristic, not
/// exhaustive — defaults to `Unknown` so we never over-penalize ambiguous
/// queries.
pub(super) fn classify_query(query: &str) -> QueryIntent {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return QueryIntent::Unknown;
    }
    if looks_like_identifier(trimmed) {
        return QueryIntent::Code;
    }
    let lower = trimmed.to_ascii_lowercase();
    // Multi-sentence prose → doc-shaped.
    let sentence_breaks = lower.matches(['.', '?', '!']).count();
    let is_multi_sentence = sentence_breaks >= 2 || lower.split_whitespace().count() > 25;

    let starts_with_any =
        |phrases: &[&str]| -> bool { phrases.iter().any(|p| lower.starts_with(p)) };
    let contains_any =
        |phrases: &[&str]| -> bool { phrases.iter().any(|p| lower.contains(p)) };

    let code_hit = starts_with_any(CODE_PHRASES) || contains_any(CODE_PHRASES);
    let doc_hit = starts_with_any(DOC_PHRASES) || contains_any(DOC_PHRASES);

    match (code_hit, doc_hit, is_multi_sentence) {
        (true, false, _) => QueryIntent::Code,
        (false, true, _) => QueryIntent::Doc,
        (false, false, true) => QueryIntent::Doc,
        _ => QueryIntent::Unknown,
    }
}

/// True when an item refers to a doc-section hit. Doc-section ids are shaped
/// `<file>::<heading>#h<level>`; symbol ids end in `#function`, `#class`, etc.
fn is_doc_section_item(item: &Value) -> bool {
    let id = item
        .pointer("/ref/id")
        .and_then(Value::as_str)
        .or_else(|| item.pointer("/ref/stable_id").and_then(Value::as_str))
        .unwrap_or("");
    let kind = id.rsplit('#').next().unwrap_or("");
    kind.starts_with('h') && kind.len() >= 2 && kind[1..].chars().all(|c| c.is_ascii_digit())
}

/// True when the caller explicitly opted into doc-section retrieval via
/// `unit_kinds=["doc_section"]` / `["section"]`. Opt-in bypasses the
/// intent-based penalty so a user who *asked for* doc sections gets
/// untouched scores.
pub(super) fn caller_opted_into_doc_sections(unit_kinds: Option<&Vec<String>>) -> bool {
    let Some(kinds) = unit_kinds else { return false };
    let only_sections = kinds
        .iter()
        .all(|k| k == "section" || k == "doc_section");
    !kinds.is_empty() && only_sections
}

/// Apply the per-intent doc-section score multiplier in place and return the
/// effective multiplier that was applied. Re-sorts items by post-multiplier
/// score (stable, descending) so the visible ordering reflects the penalty —
/// without this, fusion's symbol-then-section concatenation can keep a
/// barely-winning section ahead of a code symbol after the multiplier
/// pushes it below.
///
/// The multiplier is gap-aware for `Code` intent — see
/// [`effective_doc_section_multiplier`] — and stacks multiplicatively with
/// the vendor penalty (the `score` field is mutated in place).
/// Result of applying intent-based demotion to a result list.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) struct IntentPenalty {
    pub(super) multiplier: f64,
    /// Positions doc-sections were rank-demoted by, on top of the score
    /// multiplier. Only non-zero when intent is `Code`. Belt-and-suspenders
    /// for cases where doc raw scores are so high the multiplier alone
    /// can't pull them below the best code answer.
    pub(super) rank_demotion: usize,
}

pub(super) fn apply_intent_doc_penalty(
    items: &mut Vec<Value>,
    intent: QueryIntent,
) -> IntentPenalty {
    let mult = effective_doc_section_multiplier(intent, items);
    if (mult - 1.0).abs() >= f64::EPSILON {
        for item in items.iter_mut() {
            if !is_doc_section_item(item) {
                continue;
            }
            let Some(obj) = item.as_object_mut() else { continue };
            let old = obj.get("score").and_then(Value::as_f64).unwrap_or(0.0);
            // Round to 3 decimals to match `round_score` so JSON serialization
            // is stable.
            let new = ((old * mult) * 1000.0).round() / 1000.0;
            obj.insert("score".to_string(), serde_json::json!(new));
        }
        // Stable-sort so the multiplier actually reorders items. Without a
        // resort, fusion's symbol-then-section concatenation hides the penalty.
        items.sort_by(|a, b| {
            let sa = a.get("score").and_then(Value::as_f64).unwrap_or(0.0);
            let sb = b.get("score").and_then(Value::as_f64).unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Rank-based demotion: only for explicit Code intent. Doc-sections move
    // down by `max(N/2, 3)` positions. This is independent of score —
    // when raw doc scores are pathologically high the multiplier alone may
    // not pull them past the best code answer; this guarantees they don't
    // dominate the head of the list on a code-intent query.
    let rank_demotion = if intent == QueryIntent::Code && items.len() >= 2 {
        let d = (items.len() / 2).max(3);
        rank_demote_doc_sections(items, d);
        d
    } else {
        0
    };

    IntentPenalty {
        multiplier: mult,
        rank_demotion,
    }
}

/// Move doc-section items down by `demotion` positions, preserving relative
/// order within each class. Stable: ties in target rank break by original
/// position. Caps target at `len-1` so demoted items cluster at the tail
/// rather than going out of bounds.
fn rank_demote_doc_sections(items: &mut Vec<Value>, demotion: usize) {
    let n = items.len();
    if n < 2 || demotion == 0 {
        return;
    }
    let cap = n - 1;
    let drained = std::mem::take(items);
    let mut indexed: Vec<(usize, Value)> = drained
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let target = if is_doc_section_item(&v) {
                (i + demotion).min(cap)
            } else {
                i
            };
            (target, v)
        })
        .collect();
    indexed.sort_by_key(|(t, _)| *t);
    items.extend(indexed.into_iter().map(|(_, v)| v));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snake_case_identifier_is_code() {
        assert_eq!(classify_query("record_tool_call"), QueryIntent::Code);
    }

    #[test]
    fn camel_case_identifier_is_code() {
        assert_eq!(classify_query("CodeService"), QueryIntent::Code);
    }

    #[test]
    fn qualified_identifier_is_code() {
        assert_eq!(classify_query("foo::bar"), QueryIntent::Code);
        assert_eq!(classify_query("pkg.Type"), QueryIntent::Code);
    }

    #[test]
    fn how_does_x_work_is_code() {
        assert_eq!(
            classify_query("how does the tool track tokens saved per tool call"),
            QueryIntent::Code,
        );
        assert_eq!(
            classify_query("Where is the dispatcher implemented"),
            QueryIntent::Code,
        );
    }

    #[test]
    fn what_is_x_is_doc() {
        assert_eq!(classify_query("what is the MCP protocol"), QueryIntent::Doc);
        assert_eq!(
            classify_query("explain the indexing pipeline"),
            QueryIntent::Doc,
        );
    }

    #[test]
    fn ambiguous_short_phrase_is_unknown() {
        assert_eq!(classify_query("token tracking"), QueryIntent::Unknown);
    }

    #[test]
    fn multi_sentence_prose_is_doc() {
        let q = "I want a high-level overview. Walk me through the design choices. \
                 Why was this chosen?";
        assert_eq!(classify_query(q), QueryIntent::Doc);
    }

    #[test]
    fn opt_in_detected_for_section_only_kinds() {
        let kinds = vec!["doc_section".to_string()];
        assert!(caller_opted_into_doc_sections(Some(&kinds)));
        let kinds = vec!["section".to_string()];
        assert!(caller_opted_into_doc_sections(Some(&kinds)));
        let kinds = vec!["section".to_string(), "symbol".to_string()];
        assert!(!caller_opted_into_doc_sections(Some(&kinds)));
        assert!(!caller_opted_into_doc_sections(None));
    }

    #[test]
    fn doc_penalty_demotes_section_below_symbol_when_doc_dominates() {
        // Demo bug: doc sections raw-score 1.0 while the right code symbol
        // is at 0.522 — gap = 0.478 > 0.3, so the hard 0.4× kicks in.
        let mut items = vec![
            json!({
                "ref": { "id": ".claude/skills/foo.md::Heading#h2" },
                "score": 1.0,
            }),
            json!({
                "ref": { "id": ".claude/skills/bar.md::Other#h2" },
                "score": 0.95,
            }),
            json!({
                "ref": { "id": "src/x.rs::record_tool_call#function" },
                "score": 0.522,
            }),
        ];
        let mult = apply_intent_doc_penalty(&mut items, QueryIntent::Code).multiplier;
        assert!((mult - 0.4).abs() < 1e-9, "expected hard 0.4 multiplier, got {mult}");
        let ids: Vec<&str> = items
            .iter()
            .map(|i| i.pointer("/ref/id").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(ids[0], "src/x.rs::record_tool_call#function");
        // Doc sections demoted: 1.0 * 0.4 = 0.4, 0.95 * 0.4 = 0.38.
        let s1 = items[1].get("score").and_then(Value::as_f64).unwrap();
        let s2 = items[2].get("score").and_then(Value::as_f64).unwrap();
        assert!(s1 <= 0.4 && s2 <= 0.4);
        assert!(items[0].get("score").and_then(Value::as_f64).unwrap() > s1.max(s2));
    }

    #[test]
    fn doc_penalty_softens_when_doc_and_code_are_close() {
        // Gap = 0.6 - 0.5 = 0.1 ≤ 0.3 → 0.5× softening, not 0.4×.
        let mut items = vec![
            json!({
                "ref": { "id": "doc.md::H#h2" },
                "score": 0.6,
            }),
            json!({
                "ref": { "id": "src/x.rs::foo#function" },
                "score": 0.5,
            }),
        ];
        let mult = apply_intent_doc_penalty(&mut items, QueryIntent::Code).multiplier;
        assert!((mult - 0.5).abs() < 1e-9, "expected soft 0.5 multiplier, got {mult}");
        // Doc demoted 0.6 * 0.5 = 0.3, below the 0.5 code score.
        let s_doc = items[1].get("score").and_then(Value::as_f64).unwrap();
        assert!((s_doc - 0.3).abs() < 1e-6, "expected ~0.3, got {s_doc}");
        let ids: Vec<&str> = items
            .iter()
            .map(|i| i.pointer("/ref/id").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(ids[0], "src/x.rs::foo#function");
    }

    #[test]
    fn doc_penalty_uses_hard_multiplier_at_gap_boundary() {
        // Gap > 0.3 → hard 0.4× kicks in.
        let mut items = vec![
            json!({
                "ref": { "id": "doc.md::H#h2" },
                "score": 0.9,
            }),
            json!({
                "ref": { "id": "src/x.rs::foo#function" },
                "score": 0.5,
            }),
        ];
        let mult = apply_intent_doc_penalty(&mut items, QueryIntent::Code).multiplier;
        assert!((mult - 0.4).abs() < 1e-9, "expected hard 0.4 multiplier, got {mult}");
    }

    #[test]
    fn doc_penalty_no_code_uses_base_multiplier() {
        // No code item → no gap to compare; falls through to base 0.4.
        let mut items = vec![json!({
            "ref": { "id": "doc.md::H#h1" },
            "score": 0.5,
        })];
        let mult = apply_intent_doc_penalty(&mut items, QueryIntent::Code).multiplier;
        assert!((mult - 0.4).abs() < 1e-9, "expected base 0.4 multiplier, got {mult}");
        let s = items[0].get("score").and_then(Value::as_f64).unwrap();
        assert!((s - 0.2).abs() < 1e-6, "expected 0.5 * 0.4 = 0.2, got {s}");
    }

    #[test]
    fn doc_penalty_unknown_is_noop() {
        let mut items = vec![json!({
            "ref": { "id": "x.md::H#h1" },
            "score": 0.5,
        })];
        let p = apply_intent_doc_penalty(&mut items, QueryIntent::Unknown);
        assert!((p.multiplier - 1.0).abs() < f64::EPSILON);
        assert_eq!(p.rank_demotion, 0);
        assert_eq!(items[0].get("score").and_then(Value::as_f64), Some(0.5));
    }

    #[test]
    fn doc_intent_boosts_doc_sections() {
        let mut items = vec![json!({
            "ref": { "id": "x.md::H#h1" },
            "score": 0.4,
        })];
        let p = apply_intent_doc_penalty(&mut items, QueryIntent::Doc);
        assert!((p.multiplier - 1.15).abs() < 1e-9, "expected 1.15× boost, got {}", p.multiplier);
        assert_eq!(p.rank_demotion, 0, "doc intent should not rank-demote");
        let s = items[0].get("score").and_then(Value::as_f64).unwrap();
        assert!((s - 0.46).abs() < 1e-6, "expected ~0.46, got {s}");
    }

    #[test]
    fn rank_demotion_pushes_doc_below_low_score_code() {
        // Pathological case: doc raw score is 1.0 and code is so low that
        // even the 0.4× multiplier (→ 0.4) ties or beats the code score.
        // Rank demotion should still push the doc-section toward the tail.
        let mut items = vec![
            json!({ "ref": { "id": "doc.md::Top#h2" }, "score": 1.0 }),
            json!({ "ref": { "id": "src/a.rs::a#function" }, "score": 0.39 }),
            json!({ "ref": { "id": "src/b.rs::b#function" }, "score": 0.30 }),
            json!({ "ref": { "id": "src/c.rs::c#function" }, "score": 0.25 }),
            json!({ "ref": { "id": "src/d.rs::d#function" }, "score": 0.20 }),
        ];
        let p = apply_intent_doc_penalty(&mut items, QueryIntent::Code);
        assert_eq!(p.rank_demotion, (5 / 2usize).max(3)); // 3
        let ids: Vec<&str> = items
            .iter()
            .map(|i| i.pointer("/ref/id").and_then(Value::as_str).unwrap())
            .collect();
        // After 0.4× the doc is at 0.4, then the rank demotion pushes it
        // down 3 slots from its post-score-sort position (which is index 0
        // because 0.4 > 0.39).
        assert_eq!(ids[0], "src/a.rs::a#function");
        assert_eq!(ids.last().copied(), Some("doc.md::Top#h2"));
    }

    #[test]
    fn rank_demotion_skipped_for_doc_or_unknown() {
        let mut items = vec![
            json!({ "ref": { "id": "doc.md::Top#h2" }, "score": 0.9 }),
            json!({ "ref": { "id": "src/a.rs::a#function" }, "score": 0.5 }),
        ];
        let p = apply_intent_doc_penalty(&mut items, QueryIntent::Doc);
        assert_eq!(p.rank_demotion, 0);
    }

    #[test]
    fn rank_demotion_minimum_is_three() {
        // Even with N=2 the demotion floor is 3, so doc gets pushed past
        // every code item and lands at the tail.
        let mut items = vec![
            json!({ "ref": { "id": "doc.md::Top#h2" }, "score": 1.0 }),
            json!({ "ref": { "id": "src/a.rs::a#function" }, "score": 0.45 }),
        ];
        let p = apply_intent_doc_penalty(&mut items, QueryIntent::Code);
        assert_eq!(p.rank_demotion, 3);
        let ids: Vec<&str> = items
            .iter()
            .map(|i| i.pointer("/ref/id").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(ids[0], "src/a.rs::a#function");
        assert_eq!(ids[1], "doc.md::Top#h2");
    }
}
