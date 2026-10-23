//! Token budgeting math: greedy item-list packing by estimated JSON bytes.

use serde_json::Value;

use crate::config::BYTES_PER_TOKEN;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
