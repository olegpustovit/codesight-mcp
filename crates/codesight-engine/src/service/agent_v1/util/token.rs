//! Token budgeting math: byte-based token estimates, item packing, and
//! char-boundary-safe content truncation.

use serde_json::Value;

use crate::config::BYTES_PER_TOKEN;

/// Rough token count for ranked `total_tokens` / budget checks
/// (`BYTES_PER_TOKEN` bytes ≈ one token).
pub(in crate::service) fn token_estimate_for_context(source: &str) -> u32 {
    if source.is_empty() {
        return 0;
    }
    let b = source.len() as u64;
    let per = BYTES_PER_TOKEN;
    ((b.saturating_add(per.saturating_sub(1))) / per).clamp(1, u32::MAX as u64) as u32
}

/// Greedy pack by estimated JSON serialization bytes (`BYTES_PER_TOKEN` per budget unit).
pub(in crate::service) fn pack_catalog_items_to_token_budget(
    items: Vec<Value>,
    max_tokens: u32,
) -> (Vec<Value>, bool) {
    if max_tokens == 0 {
        return (Vec::new(), !items.is_empty());
    }
    let total = items.len();
    let budget_bytes = (max_tokens as u64).saturating_mul(BYTES_PER_TOKEN);
    let mut used: u64 = 0;
    let mut kept = Vec::with_capacity(total);
    for it in items {
        let cost = serde_json::to_string(&it)
            .map(|s| s.len() as u64)
            .unwrap_or(64);
        let cost = cost.max(1);
        if used.saturating_add(cost) > budget_bytes {
            break;
        }
        used += cost;
        kept.push(it);
    }
    let truncated = kept.len() < total;
    (kept, truncated)
}

pub(in crate::service) fn truncate_content_to_token_budget(
    content: &str,
    max_tokens: u32,
) -> (String, bool) {
    let max_bytes = (max_tokens as u64).saturating_mul(BYTES_PER_TOKEN) as usize;
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }
    let mut end = max_bytes.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (content[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_estimate_ceil_divides_and_clamps_empty() {
        assert_eq!(token_estimate_for_context(""), 0);
        assert_eq!(token_estimate_for_context("a"), 1);
        assert_eq!(token_estimate_for_context("abcd"), 1);
        assert_eq!(token_estimate_for_context("abcde"), 2);
        assert_eq!(
            token_estimate_for_context(&"x".repeat(4000)),
            (4000 / BYTES_PER_TOKEN) as u32
        );
    }

    #[test]
    fn pack_catalog_items_respects_zero_budget() {
        let items = vec![json!({"a": 1}), json!({"b": 2})];
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 0);
        assert!(kept.is_empty());
        assert!(truncated);
    }

    #[test]
    fn pack_catalog_items_zero_budget_zero_items_not_truncated() {
        let (kept, truncated) = pack_catalog_items_to_token_budget(Vec::new(), 0);
        assert!(kept.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn pack_catalog_items_stops_when_exceeding_budget() {
        let items = vec![json!({"k": "a".repeat(30)}), json!({"k": "b".repeat(30)})];
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 10);
        assert_eq!(kept.len(), 1);
        assert!(truncated);
    }

    #[test]
    fn pack_catalog_items_keeps_all_when_budget_generous() {
        let items = vec![json!({"a": 1}), json!({"b": 2})];
        let (kept, truncated) = pack_catalog_items_to_token_budget(items, 1_000);
        assert_eq!(kept.len(), 2);
        assert!(!truncated);
    }

    #[test]
    fn truncate_content_honors_char_boundary() {
        let content = "αβγδ";
        let (out, truncated) = truncate_content_to_token_budget(content, 2);
        assert_eq!(out, content);
        assert!(!truncated);
    }

    #[test]
    fn truncate_content_drops_past_budget_and_aligns_boundary() {
        let content = "αβγδ";
        let (out, truncated) = truncate_content_to_token_budget(content, 1);
        assert_eq!(out, "αβ");
        assert!(truncated);
    }

    #[test]
    fn truncate_content_rolls_back_to_valid_boundary() {
        let content = "αβγ";
        let per = BYTES_PER_TOKEN as u32;
        let (out, truncated) = truncate_content_to_token_budget(content, per + 1);
        assert!(truncated || out == content);
        assert!(content.starts_with(&out));
    }
}
