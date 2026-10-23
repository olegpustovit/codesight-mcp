//! Compose the four agent-readiness dimensions from precomputed submetrics
//! and aggregate them into a single 0–10000 signal.

use rustc_hash::{FxHashMap, FxHashSet};

use dowser_language_support::family::LanguageFamily;
use dowser_storage::storage::AnalysisSymbol;

use super::submetrics::{
    FunctionFacts, TypeAnnotationMethod, compute_change_locality, compute_hotspot_containment,
    compute_interface_encapsulation, compute_structural_localization, compute_symbol_uniqueness,
    compute_test_oracle_quality, weighted_health, weighted_type_density,
};
use super::{AgentReadinessDimensions, AgentReadinessSignal, AgentReadinessSubmetrics};

pub(super) fn agent_signal_from_inputs(
    all_symbols: &[AnalysisSymbol],
    adj: &FxHashMap<String, Vec<String>>,
    fan_in: &FxHashMap<String, usize>,
    _graph_files: &FxHashSet<String>,
    churn: &FxHashMap<String, u64>,
    facts: &[FunctionFacts<'_>],
) -> AgentReadinessSignal {
    if facts.is_empty() {
        return AgentReadinessSignal {
            score: 0.0,
            dimensions: AgentReadinessDimensions {
                navigability: 0.0,
                comprehensibility: 0.0,
                verifiability: 0.0,
                change_safety: 0.0,
            },
            submetrics: AgentReadinessSubmetrics {
                symbol_uniqueness: 0.0,
                structural_localization: 0.0,
                nesting_depth_health: 0.0,
                function_length_health: 0.0,
                type_annotation_density: 0.0,
                test_oracle_quality: 0.0,
                change_locality: 0.0,
                interface_encapsulation: 0.0,
                hotspot_containment: 0.0,
            },
            confidence: "low",
            type_annotation_method: "baseline_fallback",
            bottleneck: "navigability",
        };
    }

    let scope_graph_files: FxHashSet<String> = facts
        .iter()
        .map(|fact| fact.sym.file_path.clone())
        .collect();
    let scope_symbols: FxHashSet<&str> = facts.iter().map(|fact| fact.sym.id.as_str()).collect();
    let relevant_symbols: Vec<AnalysisSymbol> = all_symbols
        .iter()
        .filter(|sym| {
            scope_graph_files.contains(&sym.file_path) || scope_symbols.contains(sym.id.as_str())
        })
        .cloned()
        .collect();

    let submetrics = AgentReadinessSubmetrics {
        symbol_uniqueness: compute_symbol_uniqueness(&relevant_symbols, fan_in, churn),
        structural_localization: compute_structural_localization(facts),
        nesting_depth_health: weighted_health(facts, |fact| fact.nesting_depth <= 4),
        function_length_health: weighted_health(facts, |fact| {
            dowser_language_support::function_length_warn_for_path(&fact.sym.file_path)
                .is_some_and(|limit| fact.line_span <= limit)
        }),
        type_annotation_density: weighted_type_density(facts),
        test_oracle_quality: compute_test_oracle_quality(all_symbols, adj, facts),
        change_locality: compute_change_locality(&scope_graph_files, fan_in, churn),
        interface_encapsulation: compute_interface_encapsulation(&relevant_symbols),
        hotspot_containment: compute_hotspot_containment(facts),
    };

    let dimensions = AgentReadinessDimensions {
        navigability: geometric_mean(&[
            submetrics.symbol_uniqueness,
            submetrics.structural_localization,
        ]),
        comprehensibility: geometric_mean(&[
            submetrics.nesting_depth_health,
            submetrics.function_length_health,
        ]),
        verifiability: submetrics.type_annotation_density
            + (1.0 - submetrics.type_annotation_density) * submetrics.test_oracle_quality,
        change_safety: geometric_mean(&[
            submetrics.change_locality,
            submetrics.interface_encapsulation,
            submetrics.hotspot_containment,
        ]),
    };

    let score = geometric_mean(&[
        dimensions.navigability,
        dimensions.comprehensibility,
        dimensions.verifiability,
        dimensions.change_safety,
    ])
    .clamp(0.0, 1.0);

    let fallback_type_count = facts
        .iter()
        .filter(|fact| fact.type_annotation_method == TypeAnnotationMethod::BaselineFallback)
        .count();
    let measured_type_count = facts.len() - fallback_type_count;
    let graph_backed_function_files = facts
        .iter()
        .filter(|fact| fact.graph_backed)
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<FxHashSet<_>>()
        .len();
    let total_function_files = facts
        .iter()
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<FxHashSet<_>>()
        .len()
        .max(1);
    let graph_coverage_ratio = graph_backed_function_files as f64 / total_function_files as f64;
    let family_known_ratio = facts
        .iter()
        .filter(|fact| fact.family != LanguageFamily::Unknown)
        .count() as f64
        / facts.len() as f64;
    let measured_type_ratio = measured_type_count as f64 / facts.len() as f64;
    let confidence = if !churn.is_empty()
        && graph_coverage_ratio >= 0.9
        && family_known_ratio >= 0.95
        && measured_type_ratio >= 0.95
    {
        "high"
    } else if graph_coverage_ratio > 0.0 || !churn.is_empty() || measured_type_count > 0 {
        "medium"
    } else {
        "low"
    };

    let type_annotation_method = if fallback_type_count == 0 {
        TypeAnnotationMethod::Measured.as_str()
    } else {
        TypeAnnotationMethod::BaselineFallback.as_str()
    };

    AgentReadinessSignal {
        score,
        dimensions,
        submetrics,
        confidence,
        type_annotation_method,
        bottleneck: weakest_dimension(dimensions),
    }
}

fn weakest_dimension(dimensions: AgentReadinessDimensions) -> &'static str {
    [
        (dimensions.navigability, "navigability"),
        (dimensions.comprehensibility, "comprehensibility"),
        (dimensions.verifiability, "verifiability"),
        (dimensions.change_safety, "change_safety"),
    ]
    .into_iter()
    .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    .map(|(_, name)| name)
    .unwrap_or("navigability")
}

fn geometric_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let product: f64 = values.iter().product();
    product.powf(1.0 / values.len() as f64)
}
