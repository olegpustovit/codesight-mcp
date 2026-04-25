//! Per-call counterfactual baselines for `tokens_saved` (ROADMAP P1.1).
//!
//! `tokens_saved = naive_bytes_for_equivalent_work / BYTES_PER_TOKEN - response_tokens`.
//! The naive baseline varies by tool class:
//!
//! - **Structural / targeted reads** — baseline = full-file size of the file
//!   the target lives in. Used by `read_content`, `inspect_structure
//!   view=file_outline`, `assemble_context` bundle/ranked/context_bundle/
//!   section_context, and rename/diff helpers that return a single symbol.
//! - **Search** — baseline = `rg -n`-equivalent output: one
//!   `path:line:snippet\n` row per returned item, capped by the same limit
//!   the agent asked for. Not the sum of file sizes of every matched file —
//!   that's the "index dump" fantasy baseline the ROADMAP explicitly banned.
//! - **Overview / metadata** — no sensible baseline; pass `raw_bytes = 0` to
//!   `MetaBuilder::new` so the response surfaces `tokens_saved: null`
//!   (omitted key) instead of a fabricated number. Applies to
//!   `catalog_sources`, `manage_indexes`, `inspect_structure`
//!   view=repo_outline / repository_rollup / file_tree / toc*, most of
//!   `inspect_graph`, and `assemble_context` query_suggestions.
//!
//! Every baseline here is a lower bound on what the equivalent native tool
//! workflow would have cost. Overestimating inflates `tokens_saved_total` and
//! erodes agent trust; we'd rather ship an honest 15M/session than a
//! bullshit 150M.

use serde_json::Value;

use codesight_storage::storage::IndexStore;

use crate::service::helpers::{ToolError, index_error};

/// Sum of full-file sizes for every distinct code path in `refs` (one lookup
/// per path, batched). Use when the naive alternative is "read each of these
/// files in full". Missing / zero-size files contribute 0.
pub(in crate::service) fn sum_code_file_sizes<I, S>(
    store: &IndexStore,
    paths: I,
) -> Result<u64, ToolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut dedup: Vec<String> = paths
        .into_iter()
        .map(|p| p.as_ref().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    dedup.sort();
    dedup.dedup();
    if dedup.is_empty() {
        return Ok(0);
    }
    store.sum_file_sizes_for_paths(&dedup).map_err(index_error)
}

/// Approximate the bytes `rg -n` would have emitted for the same hit set.
///
/// One row per item: `path\tline\tsnippet\n` (3 separator bytes, no quoting,
/// no ANSI colour). Items with no path or snippet contribute 0. The cap is
/// intrinsically the length of `items` — the agent already asked for at most
/// `limit` hits, so we count exactly that many.
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
        // path + ':' + line + ':' + snippet + '\n'
        total = total.saturating_add(path_len + 1 + line_len + 1 + snippet_len + 1);
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
        // "src/lib.rs" (10) + ':' + "42" (2) + ':' + "fn foo() {}" (11) + '\n' = 26
        // "src/main.rs" (11) + ':' + "7" (1) + ':' + "fn main() {}" (12) + '\n' = 27
        assert_eq!(search_rg_equivalent_bytes(&items), 53);
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
