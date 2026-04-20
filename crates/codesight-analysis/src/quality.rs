//! Domain-level quality payload builders used by handlers and slice tests.

use std::collections::HashMap;

use codesight_language_support::{
    is_conventional_path, max_nesting_depth_for_path, test_weight_multiplier,
};
use serde_json::{Value, json};

use crate::agent_readiness::compute_file_churn;
use crate::complexity::{complexity_assessment, cyclomatic_complexity};
use codesight_storage::storage::{AnalysisSymbol, IndexStore};


pub fn complexity_payload(store: &IndexStore, symbol_id: &str) -> anyhow::Result<Option<Value>> {
    let Some(rec) = store.symbol_record(symbol_id)? else {
        return Ok(None);
    };

    let lines = rec
        .end_line
        .saturating_sub(rec.start_line)
        .saturating_add(1);
    let cyclomatic = cyclomatic_complexity(&rec.stored_source);
    let branch_hints = cyclomatic.saturating_sub(1);
    let first_line = rec.stored_source.lines().next().unwrap_or("");
    let param_count = count_params(first_line);
    // Report depth BELOW the function body itself: a flat function is 0,
    // a single `if` inside it is 1. `max_nesting_depth_for_file` counts
    // the function body as depth 1 (its `{` or first indent), so subtract.
    let max_nesting =
        max_nesting_depth_for_path(&rec.stored_source, &rec.file_path).saturating_sub(1);
    let assessment = complexity_assessment(cyclomatic);
    let repo_path = rec.root_path.to_string_lossy().to_string();

    Ok(Some(json!({
        "analysis": "complexity",
        "repo": repo_path,
        "file": rec.file_path,
        "line": rec.start_line,
        "findings": [{
            "finding_id": format!("complexity:{symbol_id}"),
            "analysis": "complexity",
            "severity": assessment,
            "confidence": 0.5,
            "summary": format!("cyclomatic {cyclomatic}, nesting {max_nesting}, {lines} lines"),
            "evidence": json!({
                "lines": lines,
                "branch_token_hints": branch_hints,
                "param_count": param_count,
                "max_nesting": max_nesting,
                "cyclomatic": cyclomatic,
            }),
            "related_refs": [symbol_id],
        }],
    })))
}

pub fn hotspots_payload(
    store: &IndexStore,
    min_complexity: u64,
    top_n: usize,
    include_conventional: bool,
) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_cached()?;
    let root_path = store.root_path().ok();
    let git_available = root_path
        .as_ref()
        .map(|root| root.join(".git").exists())
        .unwrap_or(false);

    let file_churn = root_path
        .as_ref()
        .map(|root| compute_file_churn(root))
        .unwrap_or_default();

    Ok(hotspots_payload_from_cached(
        &all_symbols,
        &file_churn,
        git_available,
        min_complexity,
        top_n,
        include_conventional,
    ))
}

/// Build the `analysis=hotspots` JSON payload from pre-loaded inputs.
/// Separated from [`hotspots_payload`] so other analytic tools that already
/// hold the cached symbol vec + churn map can reuse this without another
/// full-table scan.
///
/// `include_conventional`: when false (default caller behavior), files under
/// `CONVENTIONAL_DIRS` are dropped before ranking — FEEDBACK.md §4a. Test
/// files apply a 0.3× soft weight (FEEDBACK.md §4c) to keep test-quality
/// regressions visible without letting them dominate the top of the list.
pub fn hotspots_payload_from_cached(
    all_symbols: &[AnalysisSymbol],
    file_churn: &HashMap<String, u64>,
    git_available: bool,
    min_complexity: u64,
    top_n: usize,
    include_conventional: bool,
) -> Value {
    let mut hotspots = Vec::new();
    for symbol in all_symbols {
        let kind = symbol.kind.to_ascii_lowercase();
        if kind != "function" && kind != "method" {
            continue;
        }

        if !include_conventional && is_conventional_path(&symbol.file_path) {
            continue;
        }

        let cyclomatic = cyclomatic_complexity(&symbol.source_text) as u64;
        if cyclomatic < min_complexity {
            continue;
        }

        let churn = file_churn.get(&symbol.file_path).copied().unwrap_or(0);
        let test_weight = test_weight_multiplier(&symbol.file_path);
        let hotspot_score = cyclomatic as f64 * (1.0f64 + churn as f64).ln() * test_weight;
        let assessment = if !git_available || churn == 0 {
            "low"
        } else if hotspot_score > 50.0 {
            "high"
        } else if hotspot_score > 20.0 {
            "medium"
        } else {
            "low"
        };

        let rounded_score = (hotspot_score * 100.0).round() / 100.0;
        hotspots.push(json!({
            "name": symbol.name,
            "file": symbol.file_path,
            "line": symbol.start_line,
            "cyclomatic": cyclomatic,
            "churn": churn,
            "hotspot_score": rounded_score,
            "assessment": assessment,
        }));
    }

    hotspots.sort_by(|left, right| {
        let left_score = left["hotspot_score"].as_f64().unwrap_or(0.0);
        let right_score = right["hotspot_score"].as_f64().unwrap_or(0.0);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hotspots.truncate(top_n);

    json!({
        "analysis": "hotspots",
        "hotspots": hotspots,
        "git_available": git_available,
    })
}

/// Pick the files containing the top-`top_n` hottest function/method symbols
/// (ranked by `cyclomatic * log(1+churn)`). Returned order mirrors
/// [`hotspots_payload_from_cached`] — same scoring, same ranking, so the
/// "scope_files" for `refactor_targets` stay stable after the plumbing switch.
pub fn top_hotspot_files_from_cached(
    all_symbols: &[AnalysisSymbol],
    file_churn: &HashMap<String, u64>,
    top_n: usize,
    include_conventional: bool,
) -> Vec<String> {
    let mut scored: Vec<(&str, f64)> = Vec::with_capacity(all_symbols.len());
    for symbol in all_symbols {
        let kind = symbol.kind.to_ascii_lowercase();
        if kind != "function" && kind != "method" {
            continue;
        }
        if !include_conventional && is_conventional_path(&symbol.file_path) {
            continue;
        }
        let cyclomatic = cyclomatic_complexity(&symbol.source_text) as f64;
        let churn = file_churn.get(&symbol.file_path).copied().unwrap_or(0);
        let test_weight = test_weight_multiplier(&symbol.file_path);
        let score = cyclomatic * (1.0 + churn as f64).ln() * test_weight;
        scored.push((symbol.file_path.as_str(), score));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_n);
    scored.into_iter().map(|(f, _)| f.to_string()).collect()
}

pub fn extract_symbol_name_from_id(id: &str) -> String {
    let after_colons = id.rsplit("::").next().unwrap_or(id);
    after_colons
        .split('#')
        .next()
        .unwrap_or(after_colons)
        .to_string()
}

/// Count parameters in a function signature while ignoring nested generic/composite commas.
fn count_params(signature: &str) -> usize {
    let Some(open) = signature.find('(') else {
        return 0;
    };
    let after_open = open + 1;
    let bytes = signature.as_bytes();
    let mut depth: i32 = 1;
    let mut close = signature.len();
    for (i, &byte) in bytes.iter().enumerate().skip(after_open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = signature[after_open..close].trim();
    if inner.is_empty() || inner == "void" {
        return 0;
    }

    let mut params = 1usize;
    let mut bracket_depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '<' | '[' | '(' => bracket_depth += 1,
            '>' | ']' | ')' => bracket_depth -= 1,
            ',' if bracket_depth == 0 => params += 1,
            _ => {}
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::{count_params, extract_symbol_name_from_id};
    use codesight_language_support::{
        is_conventional_path, max_nesting_depth_for_path, test_weight_multiplier,
    };

    #[test]
    fn extracts_symbol_name_from_stable_id() {
        assert_eq!(
            extract_symbol_name_from_id("repo::src/lib.py::helper#function"),
            "helper"
        );
        assert_eq!(extract_symbol_name_from_id("Usage"), "Usage");
    }

    #[test]
    fn counts_nested_signature_params() {
        assert_eq!(
            count_params("fn run(a: Vec<(u8, u8)>, b: Option<String>)"),
            2
        );
        assert_eq!(count_params("void thing(void)"), 0);
    }

    #[test]
    fn computes_indentation_nesting_when_no_braces() {
        let body = "def run(x):\n    if x:\n        return 1\n    return 0\n";
        assert_eq!(max_nesting_depth_for_path(body, "test.py"), 2);
    }

    #[test]
    fn conventional_path_matches_scripts_tests_examples() {
        assert!(is_conventional_path("scripts/generate.py"));
        assert!(is_conventional_path("tests/foo.rs"));
        assert!(is_conventional_path("examples/demo.rs"));
        assert!(is_conventional_path("benches/bench_a.rs"));
        assert!(is_conventional_path("fixtures/sample.json"));
        assert!(is_conventional_path("build.rs"));
        assert!(is_conventional_path("a/b/conftest.py"));
        assert!(is_conventional_path("crates/x/tests/it.rs"));
    }

    #[test]
    fn conventional_path_lets_production_code_through() {
        assert!(!is_conventional_path("src/main.rs"));
        assert!(!is_conventional_path("crates/codesight-engine/src/service/mod.rs"));
        assert!(!is_conventional_path("app/views/user.py"));
    }

    #[test]
    fn test_weight_multiplier_downweights_test_dirs_and_suffixes() {
        assert!((test_weight_multiplier("tests/foo.rs") - 0.3).abs() < 1e-9);
        assert!((test_weight_multiplier("crates/x/tests/foo.rs") - 0.3).abs() < 1e-9);
        assert!((test_weight_multiplier("src/foo_test.py") - 0.3).abs() < 1e-9);
        assert!((test_weight_multiplier("src/foo.test.ts") - 0.3).abs() < 1e-9);
        assert!((test_weight_multiplier("src/foo_spec.rb") - 0.3).abs() < 1e-9);
    }

    #[test]
    fn test_weight_multiplier_leaves_production_at_one() {
        assert!((test_weight_multiplier("src/main.rs") - 1.0).abs() < 1e-9);
        assert!((test_weight_multiplier("crates/x/src/lib.rs") - 1.0).abs() < 1e-9);
    }
}
