//! Per-model pricing + cost-rounding helpers for the savings ledger.

use std::collections::HashMap;

/// Per-million-token input pricing (USD).
const MODEL_PRICING: &[(&str, f64)] = &[
    ("claude_opus", 15.0),
    ("claude_sonnet", 3.0),
    ("gpt4o", 2.50),
];

pub(super) fn cost_for_tokens(tokens: u64) -> HashMap<String, f64> {
    MODEL_PRICING
        .iter()
        .map(|&(model, price)| {
            let cost = (tokens as f64 / 1_000_000.0) * price;
            (model.to_string(), round_cost(cost))
        })
        .collect()
}

pub(super) fn round_cost(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub(super) fn round_cost_map(mut values: HashMap<String, f64>) -> HashMap<String, f64> {
    for value in values.values_mut() {
        *value = round_cost(*value);
    }
    values
}
