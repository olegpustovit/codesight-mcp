//! Agent-readiness scoring based on navigability, comprehensibility,
//! verifiability, and change safety.
//!
//! Per-language heuristics (visibility, type-density, family, nesting
//! mode, entry-points) live in `symslice-language-support`. This module
//! is pure scoring math on top of the façade.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use symslice_language_support::{is_test_file, is_test_symbol};
use symslice_storage::storage::AnalysisSymbol;

use crate::thresholds::current_thresholds;

mod churn;
mod signal;
mod submetrics;

pub use churn::{compute_fan_in, compute_file_churn};

use signal::agent_signal_from_inputs;
use submetrics::{FunctionFacts, build_function_facts, graph_files};

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
    /// Agent-readiness score in `[0.0, 1.0]`.
    pub score: f64,
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

pub fn is_fn_method(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("function") || kind.eq_ignore_ascii_case("method")
}

pub fn tested_file_set(
    all_symbols: &[AnalysisSymbol],
    adj: &FxHashMap<String, Vec<String>>,
) -> FxHashSet<String> {
    let mut tested: FxHashSet<String> = FxHashSet::default();

    for (src, targets) in adj {
        if is_test_file(src) {
            for tgt in targets {
                tested.insert(tgt.clone());
            }
        }
    }

    for sym in all_symbols {
        if is_test_symbol(&sym.file_path, &sym.id) {
            tested.insert(sym.file_path.clone());
        }
    }

    tested
}

/// Bundle of derived inputs the readiness scorers need. Built once via
/// [`AgentReadinessInputs::build`] so callers running both global and scoped
/// readiness in the same request share the heavy `build_function_facts`
/// walk (~hundred ms over thousands of symbol bodies) instead of paying it
/// twice.
pub struct AgentReadinessInputs<'a> {
    pub adj: &'a FxHashMap<String, Vec<String>>,
    pub all_symbols: &'a [AnalysisSymbol],
    fan_in: FxHashMap<String, usize>,
    graph_files: FxHashSet<String>,
    churn: FxHashMap<String, u64>,
    facts: Vec<FunctionFacts<'a>>,
}

impl<'a> AgentReadinessInputs<'a> {
    pub fn build(
        all_symbols: &'a [AnalysisSymbol],
        adj: &'a FxHashMap<String, Vec<String>>,
        root: Option<&Path>,
    ) -> Self {
        let churn = root.map(compute_file_churn).unwrap_or_default();
        let graph_files = graph_files(adj);
        let fan_in = compute_fan_in(adj);
        let facts = build_function_facts(all_symbols, &fan_in, &graph_files, &churn);
        Self {
            adj,
            all_symbols,
            fan_in,
            graph_files,
            churn,
            facts,
        }
    }
}

pub fn compute_agent_readiness(inputs: &AgentReadinessInputs<'_>) -> AgentReadinessSignal {
    agent_signal_from_inputs(
        inputs.all_symbols,
        inputs.adj,
        &inputs.fan_in,
        &inputs.graph_files,
        &inputs.churn,
        &inputs.facts,
    )
}

pub fn compute_agent_readiness_for_scope(
    inputs: &AgentReadinessInputs<'_>,
    scope: &str,
) -> Option<ScopedAgentReadiness> {
    let filtered_symbols: Vec<AnalysisSymbol> = inputs
        .all_symbols
        .iter()
        .filter(|sym| path_matches_scope(&sym.file_path, scope))
        .cloned()
        .collect();
    let filtered_facts: Vec<FunctionFacts<'_>> = inputs
        .facts
        .iter()
        .filter(|fact| path_matches_scope(&fact.sym.file_path, scope))
        .cloned()
        .collect();
    if filtered_symbols.is_empty() || filtered_facts.is_empty() {
        return None;
    }

    let signal = agent_signal_from_inputs(
        &filtered_symbols,
        inputs.adj,
        &inputs.fan_in,
        &inputs.graph_files,
        &inputs.churn,
        &filtered_facts,
    );
    let thresholds = current_thresholds();
    let dimensions = signal.dimensions;
    let mut thresholds_failed = Vec::new();
    for (name, value, min) in [
        (
            "navigability",
            dimensions.navigability,
            thresholds.navigability_min,
        ),
        (
            "comprehensibility",
            dimensions.comprehensibility,
            thresholds.comprehensibility_min,
        ),
        (
            "verifiability",
            dimensions.verifiability,
            thresholds.verifiability_min,
        ),
        (
            "change_safety",
            dimensions.change_safety,
            thresholds.change_safety_min,
        ),
    ] {
        if value < min {
            thresholds_failed.push(format!("{name}<{min:.2}"));
        }
    }
    let score = signal.score;
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

fn path_matches_scope(path: &str, scope: &str) -> bool {
    let trimmed = scope.trim_matches('/');
    if trimmed.is_empty() {
        return true;
    }
    path == trimmed
        || path.starts_with(&format!("{trimmed}/"))
        || path.ends_with(&format!("/{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::submetrics::compute_interface_encapsulation;
    use super::*;
    use symslice_language_support::measured_type_density_for_path;

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
        let adj = FxHashMap::default();
        let tested = tested_file_set(&syms, &adj);
        assert!(tested.contains("src/foo.rs"));
        assert!(!tested.contains("src/bar.rs"));
    }

    #[test]
    fn tested_file_set_detects_dep_graph_edges() {
        let syms = vec![];
        let adj: FxHashMap<String, Vec<String>> = [(
            "tests/integration.rs".to_string(),
            vec!["src/lib.rs".to_string()],
        )]
        .into_iter()
        .collect();
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
