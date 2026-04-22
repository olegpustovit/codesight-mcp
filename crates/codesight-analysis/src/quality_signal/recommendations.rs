//! Actionable recommendations and natural-language suggestions surfaced with
//! the governance signal.

use super::{GovernanceSignal, SignalCoverage};

pub fn suggestion_for_bottleneck(bottleneck: &str, coverage: &SignalCoverage) -> String {
    match bottleneck {
        "acyclicity" => format!(
            "Break {} dependency cycle(s) to improve score",
            coverage.cycle_count
        ),
        "modularity" => "Reduce cross-module import edges; consolidate related files".to_string(),
        "depth" => "Flatten the dependency chain; introduce intermediate facades".to_string(),
        "equality" => {
            "Reduce god-function complexity; extract helpers from high-CC functions".to_string()
        }
        "redundancy" => format!(
            "Remove {} dead file(s) and {} duplicate function(s)",
            coverage.dead_files, coverage.duplicate_functions
        ),
        "navigability" => {
            "Reduce same-name ambiguity and narrow cross-file caller spread".to_string()
        }
        "comprehensibility" => "Flatten nested code and split long functions".to_string(),
        "verifiability" => "Add real types, remove type erasure, and add focused tests".to_string(),
        "change_safety" => {
            "Reduce fan-in on hot files and tighten visibility of internal APIs".to_string()
        }
        _ => "Investigate the lowest-scoring dimension".to_string(),
    }
}

/// Build actionable recommendations from a governance signal. Each entry
/// names the weakest dimension/sub-metric, a severity, and a `first_step`
/// tool call the agent can run next.
pub(super) fn build_recommendations(gov: &GovernanceSignal) -> Vec<serde_json::Value> {
    fn severity(value: f64) -> &'static str {
        if value < 0.4 {
            "high"
        } else if value < 0.6 {
            "medium"
        } else {
            "low"
        }
    }

    let mut out: Vec<serde_json::Value> = Vec::new();
    let sm = &gov.agent.submetrics;
    let rules: &[(&str, &str, f64, &str, serde_json::Value)] = &[
        (
            "change_safety",
            "change_locality",
            sm.change_locality,
            "Reduce fan-in on hot files and split high-CC functions.",
            serde_json::json!({
                "tool": "evaluate_quality",
                "args": { "analysis": "refactor_targets" }
            }),
        ),
        (
            "change_safety",
            "hotspot_containment",
            sm.hotspot_containment,
            "Shrink the churn-weighted complexity of the hottest files.",
            serde_json::json!({
                "tool": "evaluate_quality",
                "args": { "analysis": "hotspots" }
            }),
        ),
        (
            "verifiability",
            "test_oracle_quality",
            sm.test_oracle_quality,
            "Add focused tests where coverage is thinnest.",
            serde_json::json!({
                "tool": "evaluate_quality",
                "args": { "analysis": "test_gaps" }
            }),
        ),
        (
            "navigability",
            "symbol_uniqueness",
            sm.symbol_uniqueness,
            "Resolve same-name ambiguity to make retrieval deterministic.",
            serde_json::json!({
                "tool": "search_catalog",
                "args": { "query": "", "unit_kinds": ["symbol"] }
            }),
        ),
        (
            "comprehensibility",
            "nesting_depth_health",
            sm.nesting_depth_health,
            "Flatten deeply nested blocks; split long functions.",
            serde_json::json!({
                "tool": "evaluate_quality",
                "args": { "analysis": "refactor_targets" }
            }),
        ),
    ];
    for (dim, sub, val, suggestion, first_step) in rules {
        if *val < 0.6 {
            out.push(serde_json::json!({
                "dimension": dim,
                "sub_metric": sub,
                "severity": severity(*val),
                "value": val,
                "suggestion": suggestion,
                "first_step": first_step,
            }));
        }
    }
    out.sort_by(|a, b| {
        a["value"]
            .as_f64()
            .unwrap_or(1.0)
            .partial_cmp(&b["value"].as_f64().unwrap_or(1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}
