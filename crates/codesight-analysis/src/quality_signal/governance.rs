//! Triple-layer governance signal that combines the structural quality
//! signal with agent-readiness dimensions into a single 0–10000 score.

use codesight_storage::storage::IndexStore;

use super::recommendations::build_recommendations;
use super::{QualitySignal, compute_quality_signal_precomputed, score_assessment};
use crate::agent_readiness::{AgentReadinessSignal, compute_agent_readiness};
use crate::graph::find_cycles;

/// Full triple-layer governance signal combining structural + agent readiness.
pub struct GovernanceSignal {
    /// Top-line governance score in `[0.0, 1.0]`: geometric mean of
    /// structural × agent readiness.
    pub score: f64,
    pub structural_score: f64,
    pub agent_readiness_score: f64,
    pub structural: QualitySignal,
    pub agent: AgentReadinessSignal,
    pub assessment: &'static str,
    pub confidence: &'static str,
    pub bottleneck: &'static str,
    pub bottleneck_layer: &'static str,
}

/// Compute the full triple-layer governance signal.
pub fn compute_governance_signal(store: &IndexStore) -> anyhow::Result<GovernanceSignal> {
    let adj = store.dependency_graph_strict()?;
    let cycles = find_cycles(&adj);
    let all_symbols = store.all_symbols_for_analysis()?;

    let structural = compute_quality_signal_precomputed(store, &adj, &cycles, &all_symbols)?;
    let agent = compute_agent_readiness(&all_symbols, &adj, store.root_path().ok().as_deref());
    let agent_readiness_score = agent.score;
    let structural_score = structural.score;

    let score = (structural_score * agent_readiness_score).sqrt().clamp(0.0, 1.0);

    let assessment = if score == 0.0
        && (structural.assessment == "insufficient_data" || agent.confidence == "low")
    {
        "insufficient_data"
    } else {
        score_assessment(score)
    };

    let all_dims: Vec<(f64, &str, &str)> = [
        (
            structural.root_causes.modularity,
            "modularity",
            "structural",
        ),
        (
            Some(structural.root_causes.acyclicity),
            "acyclicity",
            "structural",
        ),
        (structural.root_causes.depth, "depth", "structural"),
        (
            Some(structural.root_causes.equality),
            "equality",
            "structural",
        ),
        (
            Some(structural.root_causes.redundancy),
            "redundancy",
            "structural",
        ),
        (
            Some(agent.dimensions.navigability),
            "navigability",
            "agent_readiness",
        ),
        (
            Some(agent.dimensions.comprehensibility),
            "comprehensibility",
            "agent_readiness",
        ),
        (
            Some(agent.dimensions.verifiability),
            "verifiability",
            "agent_readiness",
        ),
        (
            Some(agent.dimensions.change_safety),
            "change_safety",
            "agent_readiness",
        ),
    ]
    .into_iter()
    .filter_map(|(v, name, layer)| v.map(|val| (val, name, layer)))
    .collect();

    let (bottleneck, bottleneck_layer) = all_dims
        .iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, name, layer)| (*name, *layer))
        .unwrap_or(("unknown", "unknown"));

    let confidence = if structural.confidence == "high" && agent.confidence == "high" {
        "high"
    } else if structural.confidence != "low" && agent.confidence != "low" {
        "medium"
    } else {
        "low"
    };

    Ok(GovernanceSignal {
        score,
        structural_score,
        agent_readiness_score,
        structural,
        agent,
        assessment,
        confidence,
        bottleneck,
        bottleneck_layer,
    })
}

pub fn governance_signal_json(gov: &GovernanceSignal, suggestion: &str) -> serde_json::Value {
    let recommendations = build_recommendations(gov);
    serde_json::json!({
        "score": gov.score,
        "structural_score": gov.structural_score,
        "agent_readiness_score": gov.agent_readiness_score,
        "assessment": gov.assessment,
        "structural_root_causes": {
            "modularity": gov.structural.root_causes.modularity,
            "acyclicity": gov.structural.root_causes.acyclicity,
            "depth": gov.structural.root_causes.depth,
            "equality": gov.structural.root_causes.equality,
            "redundancy": gov.structural.root_causes.redundancy,
        },
        "agent_dimensions": {
            "navigability": gov.agent.dimensions.navigability,
            "comprehensibility": gov.agent.dimensions.comprehensibility,
            "verifiability": gov.agent.dimensions.verifiability,
            "change_safety": gov.agent.dimensions.change_safety,
        },
        "agent_submetrics": {
            "symbol_uniqueness": gov.agent.submetrics.symbol_uniqueness,
            "structural_localization": gov.agent.submetrics.structural_localization,
            "nesting_depth_health": gov.agent.submetrics.nesting_depth_health,
            "function_length_health": gov.agent.submetrics.function_length_health,
            "type_annotation_density": gov.agent.submetrics.type_annotation_density,
            "test_oracle_quality": gov.agent.submetrics.test_oracle_quality,
            "change_locality": gov.agent.submetrics.change_locality,
            "interface_encapsulation": gov.agent.submetrics.interface_encapsulation,
            "hotspot_containment": gov.agent.submetrics.hotspot_containment,
        },
        "confidence": gov.confidence,
        "type_annotation_method": gov.agent.type_annotation_method,
        "bottleneck": gov.bottleneck,
        "bottleneck_layer": gov.bottleneck_layer,
        "suggestion": suggestion,
        "recommendations": recommendations,
    })
}
