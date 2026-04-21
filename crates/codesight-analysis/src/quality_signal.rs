//! Structural quality signal: 5 root-cause metrics aggregated into a 0–10 000
//! composite score via geometric mean. Designed for Sentrux parity.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::agent_readiness::{AgentReadinessSignal, compute_agent_readiness, is_fn_method};
use crate::complexity::cyclomatic_complexity_for_path;
use crate::dead_code;
use crate::graph::find_cycles;
use codesight_storage::storage::{AnalysisSymbol, IndexStore};

/// Per-dimension [0.0, 1.0] scores for the 5 structural root causes.
pub struct RootCauseScores {
    pub modularity: Option<f64>,
    pub acyclicity: f64,
    pub depth: Option<f64>,
    pub equality: f64,
    pub redundancy: f64,
}

pub struct QualitySignal {
    pub score: u32,
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

/// Full triple-layer governance signal combining structural + agent readiness.
pub struct GovernanceSignal {
    /// Top-line governance score (geometric mean of structural × agent readiness).
    pub score: u32,
    pub structural_score: u32,
    pub agent_readiness_score: u32,
    pub structural: QualitySignal,
    pub agent: AgentReadinessSignal,
    pub assessment: &'static str,
    pub confidence: &'static str,
    pub bottleneck: &'static str,
    pub bottleneck_layer: &'static str,
}

fn score_assessment(score: u32) -> &'static str {
    match score {
        0..=3000 => "poor",
        3001..=5000 => "fair",
        5001..=7000 => "good",
        7001..=8500 => "very_good",
        _ => "excellent",
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
    // 1. Module cohesion
    let modularity =
        compute_module_cohesion(adj).map(|cohesion| ((cohesion - 0.2) / 0.8).clamp(0.0, 1.0));

    // 2. Acyclicity
    let acyclicity = 1.0 / (1.0 + cycles.len() as f64);

    // 3. Condensed dependency depth
    let depth = condensed_dependency_depth(adj, cycles).map(|d| 1.0 / (1.0 + d as f64 / 8.0));

    // 4. Equality (Gini over per-function complexity)
    let complexities: Vec<f64> = all_symbols
        .iter()
        .filter(|s| is_fn_method(&s.kind))
        .map(|s| cyclomatic_complexity_for_path(&s.source_text, &s.file_path) as f64)
        .collect();
    let equality = if complexities.is_empty() {
        0.5 // no functions → neutral
    } else {
        (1.0 - gini_coefficient(&complexities)).clamp(0.0, 1.0)
    };

    // 5. Redundancy (dead files + duplicate functions)
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

    // Count graph coverage
    let graph_edges: usize = adj.values().map(|v| v.len()).sum();

    // Aggregate via geometric mean over available dimensions
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

    // Need at least 3 dims, at least 2 structural (non-heuristic)
    let structural_count = [modularity, Some(acyclicity), depth]
        .iter()
        .filter(|d| d.is_some())
        .count();

    if available.len() < 3 || structural_count < 2 {
        return Ok(QualitySignal {
            score: 0,
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
    let score = (signal * 10000.0).round() as u32;

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

fn top_level_module(path: &str) -> &str {
    path.split('/').next().unwrap_or(".")
}

/// Boundary-cohesion: share of dependency edges that stay within the same
/// top-level directory. Returns `None` when the graph has no usable edges.
fn compute_module_cohesion(adj: &HashMap<String, Vec<String>>) -> Option<f64> {
    let mut total = 0usize;
    let mut internal = 0usize;
    for (src, targets) in adj {
        for tgt in targets {
            if src == tgt {
                continue;
            }
            total += 1;
            if top_level_module(src) == top_level_module(tgt) {
                internal += 1;
            }
        }
    }
    (total > 0).then_some(internal as f64 / total as f64)
}

/// Compute layering depth on the SCC-condensed DAG.
fn condensed_dependency_depth(
    adj: &HashMap<String, Vec<String>>,
    cycles: &[Vec<String>],
) -> Option<usize> {
    if adj.is_empty() {
        return None;
    }

    // Map each node to its SCC index; singleton nodes get their own SCC.
    let mut node_to_scc: HashMap<&str, usize> = HashMap::new();
    let mut scc_count = 0usize;

    for scc in cycles {
        for node in scc {
            node_to_scc.insert(node.as_str(), scc_count);
        }
        scc_count += 1;
    }
    for node in adj.keys() {
        if !node_to_scc.contains_key(node.as_str()) {
            node_to_scc.insert(node.as_str(), scc_count);
            scc_count += 1;
        }
    }

    // Build condensed DAG edges
    let mut dag: HashMap<usize, Vec<usize>> = HashMap::new();
    for (src, targets) in adj {
        let src_scc = node_to_scc[src.as_str()];
        for tgt in targets {
            if let Some(&tgt_scc) = node_to_scc.get(tgt.as_str())
                && src_scc != tgt_scc
            {
                dag.entry(src_scc).or_default().push(tgt_scc);
            }
        }
    }

    // Longest path via topological sort + DP
    longest_path_in_dag(&dag, scc_count)
}

/// Longest path in a DAG using Kahn's algorithm + DP.
fn longest_path_in_dag(dag: &HashMap<usize, Vec<usize>>, node_count: usize) -> Option<usize> {
    let mut in_degree = vec![0usize; node_count];
    for targets in dag.values() {
        for &t in targets {
            if t < node_count {
                in_degree[t] += 1;
            }
        }
    }

    let mut queue: std::collections::VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|&(_, d)| *d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut dist = vec![0usize; node_count];
    let mut max_depth = 0usize;

    while let Some(u) = queue.pop_front() {
        if let Some(targets) = dag.get(&u) {
            for &v in targets {
                if v >= node_count {
                    continue;
                }
                let new_dist = dist[u] + 1;
                if new_dist > dist[v] {
                    dist[v] = new_dist;
                    max_depth = max_depth.max(new_dist);
                }
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }
    }

    Some(max_depth)
}

/// Gini coefficient over a distribution of values.
/// Returns 0.0 (perfect equality) to 1.0 (maximum inequality).
pub fn gini_coefficient(values: &[f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f64 = sorted.iter().sum();
    if sum == 0.0 {
        return 0.0;
    }
    let mut cumulative = 0.0;
    let mut area_under_lorenz = 0.0;
    for &v in &sorted {
        cumulative += v;
        area_under_lorenz += cumulative;
    }
    let gini = 1.0 - (2.0 * area_under_lorenz) / (n as f64 * sum) + 1.0 / n as f64;
    gini.clamp(0.0, 1.0)
}

/// Count functions with identical body hashes (after lightweight normalization).
fn duplicate_function_count(all_symbols: &[AnalysisSymbol]) -> usize {
    let mut body_hashes: HashMap<u64, usize> = HashMap::new();
    for sym in all_symbols {
        if !is_fn_method(&sym.kind) {
            continue;
        }
        if sym.source_text.trim().len() < 40 {
            continue; // ignore trivial bodies
        }
        let hash = hash_normalized_body(&sym.source_text);
        *body_hashes.entry(hash).or_default() += 1;
    }
    body_hashes
        .values()
        .filter(|&&c| c >= 2)
        .map(|c| c - 1)
        .sum()
}

fn hash_normalized_body(source: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let normalized: String = source
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    normalized.hash(&mut hasher);
    hasher.finish()
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

    let score = (((structural_score as f64 / 10000.0) * (agent_readiness_score as f64 / 10000.0))
        .sqrt()
        * 10000.0)
        .round() as u32;

    let assessment = if score == 0
        && (structural.assessment == "insufficient_data" || agent.confidence == "low")
    {
        "insufficient_data"
    } else {
        score_assessment(score)
    };

    // Find overall bottleneck across both layers
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

/// Shared JSON representation of a governance signal.
/// Used by both `repo_health` (embedded in a finding) and
/// `quality_signal` (as top-level response data) to avoid duplicating
/// the field layout in two handlers.
/// Build actionable recommendations from a governance signal. Each entry
/// names the weakest dimension/sub-metric, a severity, and a `first_step`
/// tool call the agent can run next.
fn build_recommendations(gov: &GovernanceSignal) -> Vec<serde_json::Value> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gini_uniform_is_zero() {
        let vals = vec![5.0, 5.0, 5.0, 5.0];
        let g = gini_coefficient(&vals);
        assert!(
            g < 0.01,
            "uniform distribution should have Gini ~0, got {g}"
        );
    }

    #[test]
    fn gini_skewed_is_high() {
        let vals = vec![0.0, 0.0, 0.0, 100.0];
        let g = gini_coefficient(&vals);
        assert!(
            g > 0.5,
            "skewed distribution should have high Gini, got {g}"
        );
    }

    #[test]
    fn gini_empty_is_zero() {
        assert_eq!(gini_coefficient(&[]), 0.0);
    }

    #[test]
    fn top_level_module_extracts_first_segment() {
        assert_eq!(top_level_module("src/lib.rs"), "src");
        assert_eq!(top_level_module("crates/core/src/lib.rs"), "crates");
        assert_eq!(top_level_module("lib.rs"), "lib.rs");
    }

    #[test]
    fn cohesion_all_internal() {
        let adj = HashMap::from([
            ("src/a.rs".to_string(), vec!["src/b.rs".to_string()]),
            ("src/b.rs".to_string(), vec!["src/c.rs".to_string()]),
        ]);
        let c = compute_module_cohesion(&adj).unwrap();
        assert!((c - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cohesion_all_external() {
        let adj = HashMap::from([
            ("src/a.rs".to_string(), vec!["tests/b.rs".to_string()]),
            ("tests/b.rs".to_string(), vec!["src/c.rs".to_string()]),
        ]);
        let c = compute_module_cohesion(&adj).unwrap();
        assert!(c < 0.01);
    }

    #[test]
    fn cohesion_empty_graph() {
        let adj: HashMap<String, Vec<String>> = HashMap::new();
        assert!(compute_module_cohesion(&adj).is_none());
    }

    #[test]
    fn depth_flat_graph() {
        let adj = HashMap::from([
            ("a".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec![]),
        ]);
        let cycles = find_cycles(&adj);
        let d = condensed_dependency_depth(&adj, &cycles).unwrap();
        assert_eq!(d, 1);
    }

    #[test]
    fn depth_empty_graph() {
        let adj: HashMap<String, Vec<String>> = HashMap::new();
        assert!(condensed_dependency_depth(&adj, &[]).is_none());
    }

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
            make_sym("c", "fn c() {}"), // too short
        ];
        assert_eq!(duplicate_function_count(&syms), 1);
    }
}
