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

/// Returns true when `first_step` is a concrete, runnable tool call — i.e.
/// not `Null`, has a non-empty `tool`, and every `args` value is either a
/// non-empty string, a non-empty array, or any other non-null JSON scalar.
/// Recommendations drop `first_step` otherwise rather than emit a no-op
/// (empty `query`, empty array) — the `suggestion` field alone is more
/// useful than a call the agent would execute and get nothing back.
fn first_step_is_actionable(step: &serde_json::Value) -> bool {
    let Some(obj) = step.as_object() else {
        return false;
    };
    let tool_ok = obj
        .get("tool")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if !tool_ok {
        return false;
    }
    let Some(args) = obj.get("args").and_then(|v| v.as_object()) else {
        // No args object is fine — some tools run without parameters.
        return true;
    };
    args.values().all(|v| match v {
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Null => false,
        _ => true,
    })
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
    // Tier 6.1: every `first_step` must point at a real tool + analysis
    // declared in `crates/dowser-engine/src/tools.json`. The wrapper
    // surface is 6 tools; `evaluate_quality` and `analysis=refactor_targets`
    // never existed. `hotspots` is the closest real analysis for
    // refactor-target suggestions.
    let rules: &[(&str, &str, f64, &str, serde_json::Value)] = &[
        (
            "change_safety",
            "change_locality",
            sm.change_locality,
            "Reduce fan-in on hot files and split high-CC functions.",
            serde_json::json!({
                "tool": "health",
                "args": { "analysis": "hotspots" }
            }),
        ),
        (
            "change_safety",
            "hotspot_containment",
            sm.hotspot_containment,
            "Shrink the churn-weighted complexity of the hottest files.",
            serde_json::json!({
                "tool": "health",
                "args": { "analysis": "hotspots" }
            }),
        ),
        (
            "verifiability",
            "test_oracle_quality",
            sm.test_oracle_quality,
            "Add focused tests where coverage is thinnest.",
            serde_json::json!({
                "tool": "health",
                "args": { "analysis": "test_gaps" }
            }),
        ),
        (
            "navigability",
            "symbol_uniqueness",
            sm.symbol_uniqueness,
            "Resolve same-name ambiguity to make retrieval deterministic.",
            // No reliable seed is available from the governance signal
            // alone; `first_step` is intentionally `Null` so the filter
            // below drops it. The `suggestion` still guides the agent.
            serde_json::Value::Null,
        ),
        (
            "comprehensibility",
            "nesting_depth_health",
            sm.nesting_depth_health,
            "Flatten deeply nested blocks; split long functions.",
            serde_json::json!({
                "tool": "health",
                "args": { "analysis": "hotspots" }
            }),
        ),
    ];
    for (dim, sub, val, suggestion, first_step) in rules {
        if *val < 0.6 {
            let mut entry = serde_json::json!({
                "dimension": dim,
                "sub_metric": sub,
                "severity": severity(*val),
                "value": val,
                "suggestion": suggestion,
            });
            if first_step_is_actionable(first_step)
                && let serde_json::Value::Object(ref mut map) = entry
            {
                map.insert("first_step".to_string(), first_step.clone());
            }
            out.push(entry);
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
