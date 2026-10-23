//! Per-call counterfactual baselines for `tokens_saved`.
//!
//! `tokens_saved = naive_bytes_for_equivalent_work / BYTES_PER_TOKEN - response_tokens`.
//!
//! - **Search** — baseline = `rg -n`-equivalent output: one
//!   `path:line:snippet\n` row per returned item, capped by the same limit
//!   the agent asked for. Not the sum of file sizes of every matched file —
//!   that's the "index dump" fantasy baseline the ROADMAP explicitly banned.
//! - **Overview / metadata** — no sensible baseline; pass `raw_bytes = 0` to
//!   `MetaBuilder::new` so the response surfaces `tokens_saved: null`
//!   (omitted key) instead of a fabricated number.
//!
//! Overestimating inflates `tokens_saved_total` and erodes agent trust;
//! we'd rather ship an honest 15M/session than a bullshit 150M.

use serde_json::Value;

/// Approximate the bytes a real "grep then read context" workflow would have
/// cost the agent for the same hit set.
///
/// Per item: `path:line:snippet\n` (the literal `rg -n` row) **plus** a
/// 5-line context window around the hit (`min(snippet_len * 5, 400)` bytes).
/// Without the context window the baseline is so tight that the JSON
/// envelope symslice returns (`{ref:{id,path,title}, score, match_type, …}`
/// ~200 bytes/hit) outweighs it and `tokens_saved` saturates to zero on
/// honest queries. The window models what `rg -n -C 2` or a follow-up `Read`
/// of the matched range would actually fetch — that's the counterfactual the
/// agent is paying compared with symslice's structured response.
///
/// Items with neither path nor snippet contribute 0.
pub(in crate::service) fn search_rg_equivalent_bytes(items: &[Value]) -> u64 {
    let mut total: u64 = 0;
    for item in items {
        let path_len = item
            .pointer("/ref/path")
            .and_then(Value::as_str)
            .map(|s| s.len())
            .unwrap_or(0) as u64;
        let line_len = item
            .pointer("/line")
            .and_then(Value::as_u64)
            .map(approximate_line_digits)
            .unwrap_or(1);
        let snippet_len = item
            .pointer("/snippet")
            .and_then(Value::as_str)
            .map(|s| s.len())
            .unwrap_or(0) as u64;
        if path_len == 0 && snippet_len == 0 {
            continue;
        }
        // Match row: path + ':' + line + ':' + snippet + '\n'
        let row_bytes = path_len + 1 + line_len + 1 + snippet_len + 1;
        // Context window: agents usually need a few lines around a match.
        // Cap at 400 bytes so a single huge snippet doesn't inflate the
        // baseline beyond what a sane `rg -C 2` / Read would return.
        let context_window = (snippet_len.saturating_mul(5)).min(400);
        total = total.saturating_add(row_bytes + context_window);
    }
    total
}

fn approximate_line_digits(line: u64) -> u64 {
    if line == 0 {
        return 1;
    }
    let mut n = line;
    let mut d = 0u64;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_rg_equivalent_bytes_approximates_rg_output() {
        let items = vec![
            json!({"ref": {"path": "src/lib.rs"}, "line": 42, "snippet": "fn foo() {}"}),
            json!({"ref": {"path": "src/main.rs"}, "line": 7, "snippet": "fn main() {}"}),
        ];
        // Row 1: "src/lib.rs"(10)+":"(1)+"42"(2)+":"(1)+"fn foo() {}"(11)+"\n"(1) = 26
        //   + window: min(11*5, 400) = 55 → 81
        // Row 2: "src/main.rs"(11)+":"(1)+"7"(1)+":"(1)+"fn main() {}"(12)+"\n"(1) = 27
        //   + window: min(12*5, 400) = 60 → 87
        assert_eq!(search_rg_equivalent_bytes(&items), 168);
    }

    #[test]
    fn search_rg_equivalent_bytes_caps_window_at_400_bytes() {
        // Snippet length 200 → 5*200 = 1000, capped to 400.
        let snippet = "x".repeat(200);
        let items = vec![json!({"ref": {"path": "f.rs"}, "line": 1, "snippet": snippet})];
        // Row: "f.rs"(4)+":"(1)+"1"(1)+":"(1)+200+"\n"(1) = 208
        //   + window: 400 → 608
        assert_eq!(search_rg_equivalent_bytes(&items), 608);
    }

    #[test]
    fn search_rg_equivalent_bytes_skips_items_without_path_or_snippet() {
        let items = vec![json!({"score": 0.5})];
        assert_eq!(search_rg_equivalent_bytes(&items), 0);
    }

    #[test]
    fn approximate_line_digits_handles_zero_and_large_numbers() {
        assert_eq!(approximate_line_digits(0), 1);
        assert_eq!(approximate_line_digits(9), 1);
        assert_eq!(approximate_line_digits(10), 2);
        assert_eq!(approximate_line_digits(999_999), 6);
    }
}
