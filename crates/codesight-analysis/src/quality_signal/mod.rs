//! Structural quality signal: 5 root-cause metrics aggregated into a 0–10 000
//! composite score via geometric mean. Designed for Sentrux parity.

use std::collections::HashMap;
use std::path::Path;

use codesight_language_support::{FALLBACK_LANGUAGE, Language, builtin_registry};
use codesight_storage::storage::{AnalysisSymbol, IndexStore};

use crate::agent_readiness::is_fn_method;
use crate::complexity::cyclomatic_complexity_for_path;
use crate::dead_code;
use crate::graph::find_cycles;

mod governance;
mod graph_math;
mod recommendations;

pub use governance::{GovernanceSignal, compute_governance_signal, governance_signal_json};
pub use graph_math::gini_coefficient;
pub use recommendations::suggestion_for_bottleneck;

use graph_math::{compute_module_cohesion, condensed_dependency_depth};

/// Per-dimension [0.0, 1.0] scores for the 5 structural root causes.
pub struct RootCauseScores {
    pub modularity: Option<f64>,
    pub acyclicity: f64,
    pub depth: Option<f64>,
    pub equality: f64,
    pub redundancy: f64,
}

pub struct QualitySignal {
    /// Structural quality, `[0.0, 1.0]`. Float everywhere — the historical
    /// int-thousandths encoding (`7402` = 0.7402) bled into JSON payloads
    /// and forced callers to remember which shape to expect.
    pub score: f64,
    pub root_causes: RootCauseScores,
    pub assessment: &'static str,
    pub confidence: &'static str,
    /// Coverage evidence for the response envelope.
    pub coverage: SignalCoverage,
    /// Lowest-scoring dimension name.
    pub bottleneck: &'static str,
}

pub struct SignalCoverage {
    pub graph_files: usize,
    pub graph_edges: usize,
    pub functions_with_source: usize,
    pub dimensions_available: usize,
    pub dead_files: usize,
    pub duplicate_functions: usize,
    pub cycle_count: usize,
}

pub(crate) fn score_assessment(score: f64) -> &'static str {
    if score <= 0.30 {
        "poor"
    } else if score <= 0.50 {
        "fair"
    } else if score <= 0.70 {
        "good"
    } else if score <= 0.85 {
        "very_good"
    } else {
        "excellent"
    }
}

/// Compute the structural quality signal, fetching all data from `store`.
pub fn compute_quality_signal(store: &IndexStore) -> anyhow::Result<QualitySignal> {
    let adj = store.dependency_graph_strict()?;
    let cycles = find_cycles(&adj);
    let all_symbols = store.all_symbols_for_analysis()?;
    compute_quality_signal_precomputed(store, &adj, &cycles, &all_symbols)
}

/// Compute the structural quality signal reusing pre-fetched graph data.
/// Used by `repo_health` to avoid duplicate queries.
pub fn compute_quality_signal_precomputed(
    store: &IndexStore,
    adj: &HashMap<String, Vec<String>>,
    cycles: &[Vec<String>],
    all_symbols: &[AnalysisSymbol],
) -> anyhow::Result<QualitySignal> {
    let modularity =
        compute_module_cohesion(adj).map(|cohesion| ((cohesion - 0.2) / 0.8).clamp(0.0, 1.0));

    let acyclicity = 1.0 / (1.0 + cycles.len() as f64);

    let depth = condensed_dependency_depth(adj, cycles).map(|d| 1.0 / (1.0 + d as f64 / 8.0));

    let complexities: Vec<f64> = all_symbols
        .iter()
        .filter(|s| is_fn_method(&s.kind))
        .map(|s| cyclomatic_complexity_for_path(&s.source_text, &s.file_path) as f64)
        .collect();
    let equality = if complexities.is_empty() {
        0.5
    } else {
        (1.0 - gini_coefficient(&complexities)).clamp(0.0, 1.0)
    };

    let total_files = store.file_count()?.max(1);
    let total_functions = complexities.len().max(1);

    let dead_data = dead_code::dead_code_payload(store, "file", 0.5, true, &[], None)?;
    let dead_files = dead_data
        .get("dead_file_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let dup_functions = duplicate_function_count(all_symbols);

    let dead_ratio = dead_files as f64 / total_files as f64;
    let dup_ratio = dup_functions as f64 / total_functions as f64;
    let redundancy = (1.0 - (0.6 * dead_ratio + 0.4 * dup_ratio)).clamp(0.0, 1.0);

    let graph_edges: usize = adj.values().map(|v| v.len()).sum();

    let dims: [(Option<f64>, &str); 5] = [
        (modularity, "modularity"),
        (Some(acyclicity), "acyclicity"),
        (depth, "depth"),
        (Some(equality), "equality"),
        (Some(redundancy), "redundancy"),
    ];
    let available: Vec<(f64, &str)> = dims
        .iter()
        .filter_map(|(v, name)| v.map(|val| (val, *name)))
        .collect();

    let dimensions_available = available.len();

    let structural_count = [modularity, Some(acyclicity), depth]
        .iter()
        .filter(|d| d.is_some())
        .count();

    if available.len() < 3 || structural_count < 2 {
        return Ok(QualitySignal {
            score: 0.0,
            root_causes: RootCauseScores {
                modularity,
                acyclicity,
                depth,
                equality,
                redundancy,
            },
            assessment: "insufficient_data",
            confidence: "low",
            coverage: SignalCoverage {
                graph_files: adj.len(),
                graph_edges,
                functions_with_source: complexities.len(),
                dimensions_available,
                dead_files,
                duplicate_functions: dup_functions,
                cycle_count: cycles.len(),
            },
            bottleneck: "unknown",
        });
    }

    let confidence = if dimensions_available >= 5 {
        "high"
    } else {
        "medium"
    };

    let product: f64 = available.iter().map(|(v, _)| v.max(0.01)).product();
    let signal = product.powf(1.0 / available.len() as f64);
    let score = signal.clamp(0.0, 1.0);

    let assessment = score_assessment(score);

    let bottleneck = available
        .iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, name)| *name)
        .unwrap_or("unknown");

    Ok(QualitySignal {
        score,
        root_causes: RootCauseScores {
            modularity,
            acyclicity,
            depth,
            equality,
            redundancy,
        },
        assessment,
        confidence,
        coverage: SignalCoverage {
            graph_files: adj.len(),
            graph_edges,
            functions_with_source: complexities.len(),
            dimensions_available,
            dead_files,
            duplicate_functions: dup_functions,
            cycle_count: cycles.len(),
        },
        bottleneck,
    })
}

/// Count functions with identical body hashes (after language-aware normalization).
fn duplicate_function_count(all_symbols: &[AnalysisSymbol]) -> usize {
    let registry = builtin_registry();
    let mut body_hashes: HashMap<u64, usize> = HashMap::new();
    for sym in all_symbols {
        if !is_fn_method(&sym.kind) {
            continue;
        }
        if sym.source_text.trim().len() < 40 {
            continue;
        }
        let normalized = match registry.for_path(Path::new(&sym.file_path)) {
            Some(lang) => lang.normalise_body_for_dedup(&sym.source_text),
            None => FALLBACK_LANGUAGE.normalise_body_for_dedup(&sym.source_text),
        };
        let digest = blake3::hash(&normalized);
        let hash =
            u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("blake3 is 32 bytes"));
        *body_hashes.entry(hash).or_default() += 1;
    }
    body_hashes
        .values()
        .filter(|&&c| c >= 2)
        .map(|c| c - 1)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_count_detects_dupes() {
        let make_sym = |name: &str, body: &str| AnalysisSymbol {
            id: name.to_string(),
            kind: "function".to_string(),
            name: name.to_string(),
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: 10,
            source_text: body.to_string(),
        };
        let long_body = "fn f() {\n    let a = 1;\n    let b = 2;\n    let c = a + b;\n    println!(\"{c}\");\n}";
        let syms = vec![
            make_sym("a", long_body),
            make_sym("b", long_body),
            make_sym("c", "fn c() {}"),
        ];
        assert_eq!(duplicate_function_count(&syms), 1);
    }
}
