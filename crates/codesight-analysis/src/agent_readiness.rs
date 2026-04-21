//! Agent-readiness scoring based on navigability, comprehensibility,
//! verifiability, and change safety.
//!
//! Per-language heuristics (visibility, type-density, family, nesting
//! mode, entry-points) live in `codesight-language-support`. This module
//! is pure scoring math on top of the façade.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use codesight_language_support::{
    fallback_type_baseline_for_path, has_restricted_visibility, is_test_file,
    language_family_for_path, max_nesting_depth_for_path, measured_type_density_for_path,
};
use codesight_language_support::family::LanguageFamily;
use codesight_storage::storage::AnalysisSymbol;

use crate::complexity::cyclomatic_complexity;
use crate::thresholds::current_thresholds;

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessDimensions {
    pub navigability: f64,
    pub comprehensibility: f64,
    pub verifiability: f64,
    pub change_safety: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessSubmetrics {
    pub symbol_uniqueness: f64,
    pub structural_localization: f64,
    pub nesting_depth_health: f64,
    pub function_length_health: f64,
    pub type_annotation_density: f64,
    pub test_oracle_quality: f64,
    pub change_locality: f64,
    pub interface_encapsulation: f64,
    pub hotspot_containment: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessSignal {
    pub score: u32,
    pub dimensions: AgentReadinessDimensions,
    pub submetrics: AgentReadinessSubmetrics,
    pub confidence: &'static str,
    pub type_annotation_method: &'static str,
    pub bottleneck: &'static str,
}

#[derive(Clone, Debug)]
pub struct FileTopOffender {
    pub symbol: String,
    pub line: u32,
    pub cyclomatic: usize,
    pub nesting_depth: usize,
}

#[derive(Clone, Debug)]
pub struct ScopedAgentReadiness {
    pub signal: AgentReadinessSignal,
    pub weakest_dimension: &'static str,
    pub thresholds_failed: Vec<String>,
    pub top_offenders: Vec<FileTopOffender>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypeAnnotationMethod {
    Measured,
    BaselineFallback,
}

impl TypeAnnotationMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::BaselineFallback => "baseline_fallback",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FunctionFacts<'a> {
    sym: &'a AnalysisSymbol,
    family: LanguageFamily,
    importance: f64,
    file_fan_in: usize,
    cyclomatic: usize,
    nesting_depth: usize,
    line_span: u32,
    type_annotation_score: f64,
    type_annotation_method: TypeAnnotationMethod,
    graph_backed: bool,
    is_test: bool,
}

pub fn is_fn_method(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("function") || kind.eq_ignore_ascii_case("method")
}

pub fn tested_file_set(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
) -> HashSet<String> {
    let mut tested: HashSet<String> = HashSet::new();

    for (src, targets) in adj {
        if is_test_file(src) {
            for tgt in targets {
                tested.insert(tgt.clone());
            }
        }
    }

    for sym in all_symbols {
        if sym.id.contains("::tests.") {
            tested.insert(sym.file_path.clone());
        }
    }

    tested
}

pub fn compute_fan_in(adj: &HashMap<String, Vec<String>>) -> HashMap<String, usize> {
    let mut fan_in: HashMap<String, usize> = HashMap::new();
    for src in adj.keys() {
        fan_in.entry(src.clone()).or_insert(0);
    }
    for targets in adj.values() {
        for tgt in targets {
            *fan_in.entry(tgt.clone()).or_insert(0) += 1;
        }
    }
    fan_in
}

pub fn compute_file_churn(root: &Path) -> HashMap<String, u64> {
    let mut churn = HashMap::<String, u64>::new();
    if !root.join(".git").exists() {
        return churn;
    }
    let Ok(output) = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only"])
        .current_dir(root)
        .output()
    else {
        return churn;
    };
    if !output.status.success() {
        return churn;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            *churn.entry(trimmed.to_string()).or_default() += 1;
        }
    }
    churn
}

pub fn compute_agent_readiness(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    root: Option<&Path>,
) -> AgentReadinessSignal {
    let churn = root.map(compute_file_churn).unwrap_or_default();
    let graph_files = graph_files(adj);
    let fan_in = compute_fan_in(adj);
    let facts = build_function_facts(all_symbols, &fan_in, &graph_files, &churn);
    agent_signal_from_inputs(all_symbols, adj, &fan_in, &graph_files, &churn, &facts)
}

pub fn compute_agent_readiness_for_scope(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    root: Option<&Path>,
    scope: &str,
) -> Option<ScopedAgentReadiness> {
    let churn = root.map(compute_file_churn).unwrap_or_default();
    let graph_files = graph_files(adj);
    let fan_in = compute_fan_in(adj);
    let facts = build_function_facts(all_symbols, &fan_in, &graph_files, &churn);
    let filtered_symbols: Vec<AnalysisSymbol> = all_symbols
        .iter()
        .filter(|sym| path_matches_scope(&sym.file_path, scope))
        .cloned()
        .collect();
    let filtered_facts: Vec<FunctionFacts<'_>> = facts
        .into_iter()
        .filter(|fact| path_matches_scope(&fact.sym.file_path, scope))
        .collect();
    if filtered_symbols.is_empty() || filtered_facts.is_empty() {
        return None;
    }

    let signal = agent_signal_from_inputs(
        &filtered_symbols,
        adj,
        &fan_in,
        &graph_files,
        &churn,
        &filtered_facts,
    );
    let thresholds = current_thresholds();
    let dimensions = signal.dimensions;
    let mut thresholds_failed = Vec::new();
    for (name, value, min) in [
        ("navigability", dimensions.navigability, thresholds.navigability_min),
        (
            "comprehensibility",
            dimensions.comprehensibility,
            thresholds.comprehensibility_min,
        ),
        ("verifiability", dimensions.verifiability, thresholds.verifiability_min),
        ("change_safety", dimensions.change_safety, thresholds.change_safety_min),
    ] {
        if value < min {
            thresholds_failed.push(format!("{name}<{min:.2}"));
        }
    }
    let score = signal.score as f64 / 10_000.0;
    if score < thresholds.agent_readiness_min {
        thresholds_failed.push(format!(
            "agent_readiness<{:.2}",
            thresholds.agent_readiness_min
        ));
    }

    let mut top_offenders: Vec<FileTopOffender> = filtered_facts
        .iter()
        .filter(|fact| !fact.is_test)
        .map(|fact| FileTopOffender {
            symbol: fact.sym.name.clone(),
            line: fact.sym.start_line,
            cyclomatic: fact.cyclomatic,
            nesting_depth: fact.nesting_depth,
        })
        .collect();
    top_offenders.sort_by(|left, right| {
        right
            .cyclomatic
            .cmp(&left.cyclomatic)
            .then(right.nesting_depth.cmp(&left.nesting_depth))
            .then(left.line.cmp(&right.line))
    });
    top_offenders.truncate(3);

    Some(ScopedAgentReadiness {
        weakest_dimension: signal.bottleneck,
        signal,
        thresholds_failed,
        top_offenders,
    })
}

fn agent_signal_from_inputs(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    fan_in: &HashMap<String, usize>,
    _graph_files: &HashSet<String>,
    churn: &HashMap<String, u64>,
    facts: &[FunctionFacts<'_>],
) -> AgentReadinessSignal {
    if facts.is_empty() {
        return AgentReadinessSignal {
            score: 0,
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

    let scope_graph_files: HashSet<String> = facts
        .iter()
        .map(|fact| fact.sym.file_path.clone())
        .collect();
    let scope_symbols: HashSet<&str> = facts.iter().map(|fact| fact.sym.id.as_str()).collect();
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
            codesight_language_support::function_length_warn_for_path(&fact.sym.file_path)
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

    let score = (geometric_mean(&[
        dimensions.navigability,
        dimensions.comprehensibility,
        dimensions.verifiability,
        dimensions.change_safety,
    ]) * 10000.0)
        .round() as u32;

    let fallback_type_count = facts
        .iter()
        .filter(|fact| fact.type_annotation_method == TypeAnnotationMethod::BaselineFallback)
        .count();
    let measured_type_count = facts.len() - fallback_type_count;
    let graph_backed_function_files = facts
        .iter()
        .filter(|fact| fact.graph_backed)
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<HashSet<_>>()
        .len();
    let total_function_files = facts
        .iter()
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<HashSet<_>>()
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

fn path_matches_scope(path: &str, scope: &str) -> bool {
    let trimmed = scope.trim_matches('/');
    if trimmed.is_empty() {
        return true;
    }
    path == trimmed || path.starts_with(&format!("{trimmed}/")) || path.ends_with(&format!("/{trimmed}"))
}

fn build_function_facts<'a>(
    all_symbols: &'a [AnalysisSymbol],
    fan_in: &HashMap<String, usize>,
    graph_files: &HashSet<String>,
    churn: &HashMap<String, u64>,
) -> Vec<FunctionFacts<'a>> {
    all_symbols
        .iter()
        .filter(|sym| is_fn_method(&sym.kind))
        .filter_map(|sym| {
            let family = language_family_for_path(&sym.file_path);
            if matches!(
                family,
                LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown
            ) {
                return None;
            }
            let file_fan_in = *fan_in.get(&sym.file_path).unwrap_or(&0);
            let file_churn = *churn.get(&sym.file_path).unwrap_or(&0);
            let importance = importance_weight(file_fan_in, file_churn);
            let cyclomatic = cyclomatic_complexity(&sym.source_text);
            let nesting_depth = max_nesting_depth_for_path(&sym.source_text, &sym.file_path);
            let line_span = sym.end_line.saturating_sub(sym.start_line) + 1;
            let (type_annotation_score, type_annotation_method) =
                measure_type_annotation(sym);

            Some(FunctionFacts {
                sym,
                family,
                importance,
                file_fan_in,
                cyclomatic,
                nesting_depth,
                line_span,
                type_annotation_score,
                type_annotation_method,
                graph_backed: graph_files.contains(&sym.file_path),
                is_test: is_test_symbol(sym),
            })
        })
        .collect()
}

fn compute_symbol_uniqueness(
    all_symbols: &[AnalysisSymbol],
    fan_in: &HashMap<String, usize>,
    churn: &HashMap<String, u64>,
) -> f64 {
    let important: Vec<&AnalysisSymbol> = all_symbols
        .iter()
        .filter(|sym| is_navigability_symbol(&sym.kind))
        .filter(|sym| !is_test_symbol(sym))
        .collect();
    if important.is_empty() {
        return 1.0;
    }

    let mut name_files: HashMap<&str, HashSet<&str>> = HashMap::new();
    for sym in &important {
        name_files
            .entry(sym.name.as_str())
            .or_default()
            .insert(sym.file_path.as_str());
    }

    let mut weighted_ambiguity_sum = 0.0;
    let mut weight_sum = 0.0;
    for sym in important {
        let importance = importance_weight(
            *fan_in.get(&sym.file_path).unwrap_or(&0),
            *churn.get(&sym.file_path).unwrap_or(&0),
        );
        let variants = name_files
            .get(sym.name.as_str())
            .map(|files| files.len())
            .unwrap_or(1);
        let ambiguity = variants.saturating_sub(1) as f64;
        weighted_ambiguity_sum += ambiguity * importance;
        weight_sum += importance;
    }

    if weight_sum == 0.0 {
        return 1.0;
    }
    let weighted_mean = weighted_ambiguity_sum / weight_sum;
    (1.0 / (1.0 + weighted_mean)).clamp(0.0, 1.0)
}

fn compute_structural_localization(facts: &[FunctionFacts<'_>]) -> f64 {
    let mut importer_counts: Vec<usize> = facts
        .iter()
        .filter(|fact| !fact.is_test && fact.graph_backed && fact.file_fan_in > 0)
        .map(|fact| fact.file_fan_in)
        .collect();
    if importer_counts.is_empty() {
        return 1.0;
    }
    importer_counts.sort_unstable();
    let mid = importer_counts.len() / 2;
    let median = if importer_counts.len().is_multiple_of(2) {
        (importer_counts[mid - 1] + importer_counts[mid]) as f64 / 2.0
    } else {
        importer_counts[mid] as f64
    };
    (1.0 / (1.0 + median / 3.0)).clamp(0.0, 1.0)
}

fn weighted_health<F>(facts: &[FunctionFacts<'_>], predicate: F) -> f64
where
    F: Fn(&FunctionFacts<'_>) -> bool,
{
    let mut weight_sum = 0.0;
    let mut healthy_weight = 0.0;
    for fact in facts {
        if fact.is_test {
            continue;
        }
        weight_sum += fact.importance;
        if predicate(fact) {
            healthy_weight += fact.importance;
        }
    }
    if weight_sum == 0.0 {
        return 1.0;
    }
    (healthy_weight / weight_sum).clamp(0.0, 1.0)
}

fn weighted_type_density(facts: &[FunctionFacts<'_>]) -> f64 {
    let mut weighted_sum = 0.0;
    let mut weight_sum = 0.0;
    for fact in facts {
        if fact.is_test {
            continue;
        }
        weighted_sum += fact.type_annotation_score * fact.importance;
        weight_sum += fact.importance;
    }
    if weight_sum == 0.0 {
        return 0.0;
    }
    (weighted_sum / weight_sum).clamp(0.0, 1.0)
}

fn compute_test_oracle_quality(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    facts: &[FunctionFacts<'_>],
) -> f64 {
    let mut source_function_counts: HashMap<&str, usize> = HashMap::new();
    let mut source_weights: HashMap<&str, f64> = HashMap::new();
    let mut inline_test_counts: HashMap<&str, usize> = HashMap::new();
    let mut external_test_counts: HashMap<String, f64> = HashMap::new();
    let mut test_file_function_counts: HashMap<&str, usize> = HashMap::new();

    for fact in facts {
        if fact.is_test {
            continue;
        }
        *source_function_counts
            .entry(fact.sym.file_path.as_str())
            .or_default() += 1;
        *source_weights
            .entry(fact.sym.file_path.as_str())
            .or_default() += fact.importance;
    }
    if source_function_counts.is_empty() {
        return 0.0;
    }

    for sym in all_symbols {
        if !is_fn_method(&sym.kind) {
            continue;
        }
        if is_test_file(&sym.file_path) {
            *test_file_function_counts
                .entry(sym.file_path.as_str())
                .or_default() += 1;
        } else if sym.id.contains("::tests.") {
            *inline_test_counts
                .entry(sym.file_path.as_str())
                .or_default() += 1;
        }
    }

    for (test_file, targets) in adj {
        if !is_test_file(test_file) {
            continue;
        }
        let test_fn_count = *test_file_function_counts
            .get(test_file.as_str())
            .unwrap_or(&0);
        if test_fn_count == 0 {
            continue;
        }
        let covered: Vec<&str> = targets
            .iter()
            .map(String::as_str)
            .filter(|target| source_function_counts.contains_key(target))
            .collect();
        if covered.is_empty() {
            continue;
        }
        let share = test_fn_count as f64 / covered.len() as f64;
        for file in covered {
            *external_test_counts.entry(file.to_string()).or_default() += share;
        }
    }

    let mut weighted_sum = 0.0;
    let mut weight_sum = 0.0;
    for (file, fn_count) in source_function_counts {
        let inline = *inline_test_counts.get(file).unwrap_or(&0) as f64;
        let external = *external_test_counts.get(file).unwrap_or(&0.0);
        let ratio = ((inline + external) / fn_count as f64).min(1.0);
        let weight = *source_weights.get(file).unwrap_or(&1.0);
        weighted_sum += ratio * weight;
        weight_sum += weight;
    }
    if weight_sum == 0.0 {
        return 0.0;
    }
    (weighted_sum / weight_sum).clamp(0.0, 1.0)
}

fn compute_change_locality(
    graph_files: &HashSet<String>,
    fan_in: &HashMap<String, usize>,
    churn: &HashMap<String, u64>,
) -> f64 {
    if graph_files.is_empty() {
        return 1.0;
    }

    let mut weighted_surface = 0.0;
    let mut file_count = 0usize;

    for file in graph_files {
        let importers = *fan_in.get(file).unwrap_or(&0) as f64;
        let file_churn = *churn.get(file).unwrap_or(&0) as f64;
        if importers < 2.0 && file_churn < 5.0 {
            continue;
        }
        let surface = (1.0 + importers) * (1.0 + (1.0 + file_churn).ln());
        weighted_surface += surface;
        file_count += 1;
    }

    if file_count == 0 {
        return 1.0;
    }

    let avg_surface = weighted_surface / file_count as f64;
    (1.0 / (1.0 + avg_surface / 8.0)).clamp(0.0, 1.0)
}

fn compute_interface_encapsulation(all_symbols: &[AnalysisSymbol]) -> f64 {
    let mut total = 0usize;
    let mut restricted = 0usize;
    for sym in all_symbols {
        if is_test_symbol(sym) {
            continue;
        }
        total += 1;
        if has_restricted_visibility(&sym.file_path, &sym.source_text, &sym.name) {
            restricted += 1;
        }
    }
    if total == 0 {
        return 1.0;
    }
    (restricted as f64 / total as f64).clamp(0.0, 1.0)
}

fn compute_hotspot_containment(facts: &[FunctionFacts<'_>]) -> f64 {
    let hotspots: Vec<&FunctionFacts<'_>> = facts
        .iter()
        .filter(|fact| !fact.is_test && fact.graph_backed && fact.cyclomatic > 15)
        .collect();
    if hotspots.is_empty() {
        return 1.0;
    }
    let contained = hotspots.iter().filter(|fact| fact.file_fan_in <= 5).count();
    (contained as f64 / hotspots.len() as f64).clamp(0.0, 1.0)
}

fn graph_files(adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut files = HashSet::new();
    for (src, targets) in adj {
        files.insert(src.clone());
        for tgt in targets {
            files.insert(tgt.clone());
        }
    }
    files
}

fn importance_weight(fan_in: usize, churn: u64) -> f64 {
    (1.0 + fan_in as f64) * (1.0 + (1.0 + churn as f64).ln())
}

fn geometric_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let product: f64 = values.iter().product();
    product.powf(1.0 / values.len() as f64)
}

fn is_navigability_symbol(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "function" | "method" | "class" | "struct" | "trait" | "interface" | "enum"
    )
}

fn is_test_symbol(sym: &AnalysisSymbol) -> bool {
    is_test_file(&sym.file_path) || sym.id.contains("::tests.")
}

/// Compute nesting depth for `source` given the file path. Exposed for the
/// per-symbol complexity payload in `quality.rs`.
pub fn max_nesting_depth_for_file(source: &str, file_path: &str) -> usize {
    max_nesting_depth_for_path(source, file_path)
}

fn measure_type_annotation(sym: &AnalysisSymbol) -> (f64, TypeAnnotationMethod) {
    if let Some(score) = measured_type_density_for_path(&sym.file_path, &sym.source_text) {
        return (score, TypeAnnotationMethod::Measured);
    }
    (
        fallback_type_baseline_for_path(&sym.file_path).unwrap_or(0.0),
        TypeAnnotationMethod::BaselineFallback,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(id: &str, file: &str, source: &str) -> AnalysisSymbol {
        AnalysisSymbol {
            id: id.to_string(),
            kind: "function".to_string(),
            name: id.split("::").last().unwrap_or("f").to_string(),
            file_path: file.to_string(),
            start_line: 1,
            end_line: source.lines().count() as u32,
            source_text: source.to_string(),
        }
    }

    #[test]
    fn rust_unit_return_counts_as_typed() {
        let source = "fn run(&self, value: i32) {\n    if value > 1 {\n        println!(\"{value}\");\n    }\n}";
        let score = measured_type_density_for_path("lib.rs", source).unwrap();
        assert!((score - 1.0).abs() < 0.001, "unexpected score {score}");
    }

    #[test]
    fn rust_dyn_any_is_penalized() {
        let source = "fn erase(value: Box<dyn Any>) -> Box<dyn Any> { value }";
        let score = measured_type_density_for_path("lib.rs", source).unwrap();
        assert!(score < 0.1, "dyn Any should score near zero, got {score}");
    }

    #[test]
    fn python_any_and_missing_annotations_are_penalized() {
        let source = "def foo(x: Any, y) -> None:\n    return None\n";
        let score = measured_type_density_for_path("lib.py", source).unwrap();
        assert!(
            (score - (1.0 / 3.0)).abs() < 0.05,
            "unexpected score {score}"
        );
    }

    #[test]
    fn go_grouped_parameters_and_void_return_are_typed() {
        let source = "func foo(x, y int) {\n    fmt.Println(x + y)\n}";
        let score = measured_type_density_for_path("main.go", source).unwrap();
        assert!((score - 1.0).abs() < 0.001, "unexpected score {score}");
    }

    #[test]
    fn javascript_without_jsdoc_scores_zero() {
        let source = "function foo(x, y) {\n  return x + y;\n}";
        let score = measured_type_density_for_path("app.js", source).unwrap();
        assert!(score < 0.01, "unexpected score {score}");
    }

    #[test]
    fn tested_file_set_detects_inline_tests() {
        let syms = vec![
            sym(
                "src/foo.rs::do_work#function",
                "src/foo.rs",
                "fn do_work() {}",
            ),
            sym(
                "src/foo.rs::tests.test_do_work#function",
                "src/foo.rs",
                "fn test_do_work() {}",
            ),
            sym(
                "src/bar.rs::handle#function",
                "src/bar.rs",
                "fn handle() {}",
            ),
        ];
        let adj = HashMap::new();
        let tested = tested_file_set(&syms, &adj);
        assert!(tested.contains("src/foo.rs"));
        assert!(!tested.contains("src/bar.rs"));
    }

    #[test]
    fn tested_file_set_detects_dep_graph_edges() {
        let syms = vec![];
        let adj = HashMap::from([(
            "tests/integration.rs".to_string(),
            vec!["src/lib.rs".to_string()],
        )]);
        let tested = tested_file_set(&syms, &adj);
        assert!(tested.contains("src/lib.rs"));
    }

    #[test]
    fn verifiability_substitute_model() {
        let typed_no_tests = make_verifiability(1.0, 0.0);
        assert!((typed_no_tests - 1.0).abs() < 0.001, "got {typed_no_tests}");

        let untyped_good_tests = make_verifiability(0.0, 0.8);
        assert!(
            (untyped_good_tests - 0.8).abs() < 0.001,
            "got {untyped_good_tests}"
        );

        let partial = make_verifiability(0.7, 0.5);
        assert!((partial - 0.85).abs() < 0.001, "got {partial}");

        let nothing = make_verifiability(0.0, 0.0);
        assert!(nothing < 0.001, "got {nothing}");
    }

    fn make_verifiability(type_density: f64, oracle: f64) -> f64 {
        type_density + (1.0 - type_density) * oracle
    }

    #[test]
    fn interface_encapsulation_respects_rust_visibility() {
        let syms = vec![
            sym("a", "src/lib.rs", "pub fn public_api() {}"),
            sym("b", "src/lib.rs", "fn private_impl() {}"),
        ];
        let score = compute_interface_encapsulation(&syms);
        assert!((score - 0.5).abs() < 0.001, "unexpected score {score}");
    }
}
