//! Dead-code analysis backed by named evidence providers.

mod entrypoints;
mod reachability;

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::{
    analysis::{evidence::EvidenceNote, graph as dep_graph},
    storage::{AnalysisSymbol, IndexStore},
};

use self::entrypoints::{is_test_file, is_test_fixture};
use self::reachability::build_repo_reachability;

/// Build the `data` object for `evaluate_quality` with `analysis: "dead_code"`.
///
/// `limit` caps the `dead_files` / `dead_symbols` arrays in the response
/// (full counts are always reported via `dead_file_count` / `dead_symbol_count`).
/// `None` means unbounded, preserving existing callers that pass through the
/// internal `repo_health` → `dead_code` integration.
pub fn dead_code_payload(
    store: &IndexStore,
    granularity: &str,
    min_confidence: f64,
    include_tests: bool,
    entry_patterns: &[String],
    limit: Option<usize>,
) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_for_analysis()?;
    let dep_graph = store.dependency_graph_strict()?;
    let all_files = store.all_file_paths()?;
    let root_path = store.root_path().ok();

    // Files that actually contain executable code. A file with only
    // JSON keys (tags.scm captures them as `constant`) or other data-only
    // symbols has no place in a reachability graph — otherwise every
    // `.mcp.json` / `package.json` shows up as "dead code" forever.
    // Only files with at least one function/method/class/trait/struct/enum
    // qualify.
    let is_executable = |kind: &str| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "function" | "method" | "class" | "trait" | "struct" | "enum" | "type" | "module"
        )
    };
    let files_with_code: HashSet<&str> = all_symbols
        .iter()
        .filter(|s| is_executable(&s.kind))
        .map(|s| s.file_path.as_str())
        .collect();

    let reachability = build_repo_reachability(
        &all_files,
        &dep_graph,
        &all_symbols,
        root_path.as_deref(),
        include_tests,
        entry_patterns,
    );

    let mut dead_files: Vec<Value> = Vec::new();
    for file in &all_files {
        if is_test_fixture(file) {
            continue;
        }
        if !include_tests && is_test_file(file) {
            continue;
        }
        if reachability.reachable_files.contains(file) {
            continue;
        }
        if !files_with_code.contains(file.as_str()) {
            continue;
        }

        let confidence = if dep_graph
            .get(file)
            .is_some_and(|targets| !targets.is_empty())
        {
            0.7
        } else {
            1.0
        };
        if confidence < min_confidence {
            continue;
        }

        dead_files.push(json!({
            "file": file,
            "confidence": confidence,
            "signal": "unreachable_from_live_roots",
        }));
    }

    let mut dead_symbols: Vec<Value> = Vec::new();
    if granularity == "symbol" {
        let dead_file_set: HashSet<&str> = dead_files
            .iter()
            .filter_map(|dead_file| dead_file["file"].as_str())
            .collect();
        for symbol in &all_symbols {
            if is_test_fixture(&symbol.file_path) {
                continue;
            }
            if !include_tests && is_test_file(&symbol.file_path) {
                continue;
            }
            if !dead_file_set.contains(symbol.file_path.as_str()) {
                continue;
            }
            dead_symbols.push(json!({
                "symbol_id": symbol.id,
                "name": symbol.name,
                "kind": symbol.kind,
                "file": symbol.file_path,
                "line": symbol.start_line,
                "confidence": 0.8,
                "signal": "dead_file",
            }));
        }
    }

    let dead_file_count = dead_files.len();
    let dead_symbol_count = dead_symbols.len();
    let truncated = match limit {
        Some(cap) => {
            let need = dead_files.len() > cap || dead_symbols.len() > cap;
            dead_files.truncate(cap);
            dead_symbols.truncate(cap);
            need
        }
        None => false,
    };
    let mut data = json!({
        "analysis": "dead_code",
        "dead_files": dead_files,
        "dead_file_count": dead_file_count,
        "dead_symbols": if granularity == "symbol" { json!(dead_symbols) } else { json!([]) },
        "dead_symbol_count": if granularity == "symbol" { dead_symbol_count } else { 0 },
        "live_root_count": reachability.live_roots.len(),
        "truncated": truncated,
        "analysis_notes": "dead code detection uses dependency-graph reachability plus labeled entry-root heuristics",
    });

    if let Some(framework_warning) = reachability.framework_warning {
        data["framework_warning"] = json!(framework_warning);
    }

    Ok(data)
}

/// Build the `data` object for `evaluate_quality` with `analysis: "dead_code_v2"`.
pub fn dead_code_v2_payload(store: &IndexStore, min_confidence: f64) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_for_analysis()?;
    let dep_graph = store.dependency_graph_strict()?;
    let all_files = store.all_file_paths()?;
    let root_path = store.root_path().ok();
    let all_file_contents = store.all_file_contents().unwrap_or_default();

    let reachability = build_repo_reachability(
        &all_files,
        &dep_graph,
        &all_symbols,
        root_path.as_deref(),
        false,
        &[],
    );

    let mut dead_symbols: Vec<Value> = Vec::new();
    for symbol in &all_symbols {
        if is_test_fixture(&symbol.file_path) || is_test_file(&symbol.file_path) {
            continue;
        }
        if reachability.explicit_live_roots.contains(&symbol.file_path) {
            continue;
        }

        let mut signals: Vec<&str> = Vec::new();
        let mut evidence_notes = Vec::new();
        let mut confidence: f64 = 0.0;

        if !reachability.reachable_files.contains(&symbol.file_path) {
            signals.push("unreachable_file");
            evidence_notes.push(EvidenceNote::structural(
                "unreachable_file",
                "containing file is not reachable from seeded live roots",
            ));
            confidence += 0.55;
        }

        let ref_count = store.symbol_reference_count(&symbol.name, &symbol.id)?;
        if ref_count == 0 {
            signals.push("no_callers");
            evidence_notes.push(EvidenceNote::mixed(
                "no_callers",
                "no indexed symbol spans reference this symbol name",
            ));
            confidence += 0.25;
        }

        if !is_barrel_exported(symbol, &all_file_contents) && !signals.is_empty() {
            signals.push("not_barrel_exported");
            evidence_notes.push(EvidenceNote::heuristic(
                "not_barrel_exported",
                "symbol name was not found in package barrel files such as __init__.py",
            ));
            confidence += 0.20;
        }

        confidence = confidence.min(1.0);
        if signals.is_empty() || confidence < min_confidence {
            continue;
        }

        dead_symbols.push(json!({
            "id": symbol.id,
            "name": symbol.name,
            "kind": symbol.kind,
            "file": symbol.file_path,
            "line": symbol.start_line,
            "confidence": confidence,
            "signals": signals,
            "evidence_notes": serialize_notes(&evidence_notes),
        }));
    }

    let total_analysed = all_symbols
        .iter()
        .filter(|symbol| !is_test_file(&symbol.file_path) && !is_test_fixture(&symbol.file_path))
        .count();
    let dead_symbol_count = dead_symbols.len();

    Ok(json!({
        "analysis": "dead_code_v2",
        "dead_symbols": dead_symbols,
        "dead_symbol_count": dead_symbol_count,
        "total_analysed": total_analysed,
        "analysis_notes": "dead_code_v2 combines file reachability, indexed symbol references, and barrel-export hints",
        "evidence_notes": serialize_notes(&reachability.evidence_notes),
    }))
}

fn is_barrel_exported(symbol: &AnalysisSymbol, all_file_contents: &[(String, String)]) -> bool {
    all_file_contents.iter().any(|(file, content)| {
        file.ends_with("__init__.py")
            && file != &symbol.file_path
            && dep_graph::word_match(content, &symbol.name)
    })
}

fn serialize_notes(notes: &[EvidenceNote]) -> Value {
    serde_json::to_value(notes).unwrap_or_else(|_| json!([]))
}

/// Trim `dead_files` and `dead_symbols` in an already-built `dead_code`
/// payload so the serialized response fits under `max_tokens` (≈4 bytes per
/// token). Sets `truncated: true` when anything was dropped. Used by the
/// `evaluate_quality` handler when callers pass `max_tokens`.
pub fn pack_dead_code_to_budget(data: &mut Value, max_tokens: u32) {
    let byte_budget = (max_tokens as usize).saturating_mul(crate::config::BYTES_PER_TOKEN as usize);
    if serialized_len(data) <= byte_budget {
        return;
    }
    let mut any_dropped = false;
    loop {
        let files_len = data
            .get("dead_files")
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        let syms_len = data
            .get("dead_symbols")
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        if files_len == 0 && syms_len == 0 {
            break;
        }
        if syms_len >= files_len {
            if let Some(arr) = data.get_mut("dead_symbols").and_then(|v| v.as_array_mut()) {
                arr.pop();
            }
        } else if let Some(arr) = data.get_mut("dead_files").and_then(|v| v.as_array_mut()) {
            arr.pop();
        }
        any_dropped = true;
        if serialized_len(data) <= byte_budget {
            break;
        }
    }
    if any_dropped {
        data["truncated"] = json!(true);
    }
}

fn serialized_len(v: &Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)
}
