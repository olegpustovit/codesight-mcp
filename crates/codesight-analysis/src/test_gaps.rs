//! Test gap proxy: rank code without an indexed test-file dependency by
//! `cyclomatic × (1 + fan_in) × (1 + ln(1 + churn))`.

use rustc_hash::{FxHashMap, FxHashSet};

use serde_json::{Value, json};

use codesight_language_support::is_test_file;

use crate::agent_readiness::{compute_fan_in, compute_file_churn, is_fn_method, tested_file_set};
use crate::complexity::cyclomatic_complexity_for_path;
use codesight_storage::storage::{AnalysisSymbol, IndexStore};

pub struct TestGap {
    pub symbol_id: String,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub risk_score: f64,
    pub cyclomatic: usize,
    pub fan_in: usize,
    pub churn: u64,
    pub reason: String,
}

/// Rank symbols lacking a recognized indexed test-file dependency (not executed coverage).
///
/// Returns the JSON `data` payload for the MCP response.
pub fn test_gap_payload(
    store: &IndexStore,
    top_n: usize,
    min_complexity: u64,
) -> anyhow::Result<Value> {
    let adj = store.dependency_graph_strict()?;
    let all_symbols = store.all_symbols_for_analysis()?;
    let root = store.root_path().ok();
    let churn = root
        .as_ref()
        .map(|r| compute_file_churn(r))
        .unwrap_or_default();

    let fan_in = compute_fan_in(&adj);
    let tested = tested_file_set(&all_symbols, &adj);

    let mut gaps = compute_gaps(&all_symbols, &tested, &fan_in, &churn, min_complexity);

    let total_untested = gaps.len();
    gaps.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    gaps.truncate(top_n);

    let items: Vec<Value> = gaps
        .iter()
        .map(|g| {
            json!({
                "symbol_id": g.symbol_id,
                "name": g.name,
                "file": g.file,
                "line": g.line,
                "risk_score": g.risk_score,
                "cyclomatic": g.cyclomatic,
                "fan_in": g.fan_in,
                "churn": g.churn,
                "reason": g.reason,
            })
        })
        .collect();

    Ok(json!({
        "analysis": "test_gaps",
        "total_untested_functions": total_untested,
        "evidence": {
            "basis":"heuristic_test_file_dependency_graph",
            "executed_test_coverage":false,
            "total_untested_functions_is_proxy":true,
            "meaning":"Candidates lack a recognized test-file dependency path in the index; this does not establish absence of tests or behavioral coverage. The count includes only candidates meeting the minimum complexity threshold."
        },
        "gaps": items,
    }))
}

fn compute_gaps(
    all_symbols: &[AnalysisSymbol],
    tested: &FxHashSet<String>,
    fan_in: &FxHashMap<String, usize>,
    churn: &FxHashMap<String, u64>,
    min_complexity: u64,
) -> Vec<TestGap> {
    let mut gaps = Vec::new();

    for sym in all_symbols {
        if !is_fn_method(&sym.kind) {
            continue;
        }
        if is_test_file(&sym.file_path) {
            continue;
        }
        if tested.contains(&sym.file_path) {
            continue;
        }

        let cc = cyclomatic_complexity_for_path(&sym.source_text, &sym.file_path);
        if (cc as u64) < min_complexity {
            continue;
        }

        let fi = *fan_in.get(&sym.file_path).unwrap_or(&0);
        let ch = *churn.get(&sym.file_path).unwrap_or(&0);
        let risk = cc as f64 * (1.0 + fi as f64) * (1.0 + (1.0 + ch as f64).ln());

        gaps.push(TestGap {
            symbol_id: sym.id.clone(),
            name: sym.name.clone(),
            file: sym.file_path.clone(),
            line: sym.start_line,
            risk_score: risk,
            cyclomatic: cc,
            fan_in: fi,
            churn: ch,
            reason: format!("no indexed test dependency, cc={cc}, fan_in={fi}, churn={ch}"),
        });
    }

    gaps
}
