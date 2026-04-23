//! Submetric calculators + `FunctionFacts` builder feeding the agent-readiness
//! signal. Every call here is per-function math over language-support facade
//! outputs; the orchestration lives in `signal.rs`.

use std::collections::{HashMap, HashSet};

use codesight_language_support::family::LanguageFamily;
use codesight_language_support::{
    fallback_type_baseline_for_path, has_restricted_visibility,
    idiomatic_symbol_names_for_path, is_test_file, language_family_for_path,
    max_nesting_depth_for_path, measured_type_density_for_path,
};
use codesight_storage::storage::AnalysisSymbol;

use super::is_language_test_id;
use crate::agent_readiness::is_fn_method;
use crate::complexity::cyclomatic_complexity_for_path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TypeAnnotationMethod {
    Measured,
    BaselineFallback,
}

impl TypeAnnotationMethod {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::BaselineFallback => "baseline_fallback",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FunctionFacts<'a> {
    pub(super) sym: &'a AnalysisSymbol,
    pub(super) family: LanguageFamily,
    pub(super) importance: f64,
    pub(super) file_fan_in: usize,
    pub(super) cyclomatic: usize,
    pub(super) nesting_depth: usize,
    pub(super) line_span: u32,
    pub(super) type_annotation_score: f64,
    pub(super) type_annotation_method: TypeAnnotationMethod,
    pub(super) graph_backed: bool,
    pub(super) is_test: bool,
}

pub(super) fn graph_files(adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut files = HashSet::new();
    for (src, targets) in adj {
        files.insert(src.clone());
        for tgt in targets {
            files.insert(tgt.clone());
        }
    }
    files
}

pub(super) fn importance_weight(fan_in: usize, churn: u64) -> f64 {
    (1.0 + fan_in as f64) * (1.0 + (1.0 + churn as f64).ln())
}

pub(super) fn build_function_facts<'a>(
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
            let cyclomatic = cyclomatic_complexity_for_path(&sym.source_text, &sym.file_path);
            let nesting_depth = max_nesting_depth_for_path(&sym.source_text, &sym.file_path);
            let line_span = sym.end_line.saturating_sub(sym.start_line) + 1;
            let (type_annotation_score, type_annotation_method) = measure_type_annotation(sym);

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

/// Whether `sym.name` is idiomatic for the language that owns `sym.file_path`.
/// Idiomatic names (Rust `new`, Python `__init__`, Java `equals`, Go `String`,
/// …) repeat across files by language convention and a competent reader
/// disambiguates them by owning type — they should not count toward the
/// `symbol_uniqueness` collision tally. Language-owned list lives on the
/// `Language` trait (L0.75); this function is the L1 dispatch.
fn is_idiomatic_for_language(sym: &AnalysisSymbol) -> bool {
    idiomatic_symbol_names_for_path(&sym.file_path).contains(&sym.name.as_str())
}

pub(super) fn compute_symbol_uniqueness(
    all_symbols: &[AnalysisSymbol],
    fan_in: &HashMap<String, usize>,
    churn: &HashMap<String, u64>,
) -> f64 {
    let important: Vec<&AnalysisSymbol> = all_symbols
        .iter()
        .filter(|sym| is_navigability_symbol(&sym.kind))
        .filter(|sym| !is_test_symbol(sym))
        .filter(|sym| !is_idiomatic_for_language(sym))
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

pub(super) fn compute_structural_localization(facts: &[FunctionFacts<'_>]) -> f64 {
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

pub(super) fn weighted_health<F>(facts: &[FunctionFacts<'_>], predicate: F) -> f64
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

pub(super) fn weighted_type_density(facts: &[FunctionFacts<'_>]) -> f64 {
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

pub(super) fn compute_test_oracle_quality(
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
        } else if is_language_test_id(sym) {
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

pub(super) fn compute_change_locality(
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

pub(super) fn compute_interface_encapsulation(all_symbols: &[AnalysisSymbol]) -> f64 {
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

pub(super) fn compute_hotspot_containment(facts: &[FunctionFacts<'_>]) -> f64 {
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

fn is_navigability_symbol(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "function" | "method" | "class" | "struct" | "trait" | "interface" | "enum"
    )
}

pub(super) fn is_test_symbol(sym: &AnalysisSymbol) -> bool {
    is_test_file(&sym.file_path) || is_language_test_id(sym)
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
mod idiomatic_exemption_tests {
    use super::*;

    fn sym(file: &str, name: &str) -> AnalysisSymbol {
        AnalysisSymbol {
            id: format!("{file}::{name}#function"),
            kind: "function".into(),
            name: name.into(),
            file_path: file.into(),
            start_line: 1,
            end_line: 5,
            source_text: String::new(),
        }
    }

    #[test]
    fn rust_fn_new_collision_does_not_drag_uniqueness_down() {
        // 50 files each with a `fn new`. Without the exemption, variants=50
        // for every "new" symbol and the score collapses to near 0.
        let syms: Vec<AnalysisSymbol> = (0..50)
            .map(|i| sym(&format!("src/mod_{i}.rs"), "new"))
            .collect();
        let score = compute_symbol_uniqueness(&syms, &HashMap::new(), &HashMap::new());
        assert!(
            score >= 0.99,
            "expected ~1.0 when every symbol is idiomatic Rust `new`, got {score}"
        );
    }

    #[test]
    fn python_dunder_collisions_are_exempt() {
        let syms: Vec<AnalysisSymbol> = (0..40)
            .map(|i| sym(&format!("pkg/mod_{i}.py"), "__init__"))
            .collect();
        let score = compute_symbol_uniqueness(&syms, &HashMap::new(), &HashMap::new());
        assert!(
            score >= 0.99,
            "expected ~1.0 when every symbol is Python `__init__`, got {score}"
        );
    }

    #[test]
    fn non_idiomatic_collisions_still_penalise() {
        // 20 Rust files each defining `fn frobnicate` — not idiomatic.
        let syms: Vec<AnalysisSymbol> = (0..20)
            .map(|i| sym(&format!("src/mod_{i}.rs"), "frobnicate"))
            .collect();
        let score = compute_symbol_uniqueness(&syms, &HashMap::new(), &HashMap::new());
        assert!(
            score < 0.1,
            "expected heavy penalty for 20-way collision on non-idiomatic name, got {score}"
        );
    }

    #[test]
    fn exemption_is_per_language_not_global() {
        // `initialize` is idiomatic Ruby, not Rust. A Rust file with
        // `fn initialize` should not get the free pass Ruby gets.
        let syms = vec![
            sym("lib/a.rb", "initialize"),
            sym("lib/b.rb", "initialize"),
            sym("lib/c.rb", "initialize"),
            sym("src/x.rs", "initialize"),
            sym("src/y.rs", "initialize"),
            sym("src/z.rs", "initialize"),
        ];
        // The Ruby trio is exempt; the Rust trio still collides 3-way.
        // Variants-minus-1 = 2 on three remaining files; score = 1/(1+2)=0.33.
        let score = compute_symbol_uniqueness(&syms, &HashMap::new(), &HashMap::new());
        assert!(
            (0.3..0.4).contains(&score),
            "expected Rust-only collision around 0.33, got {score}"
        );
    }
}
