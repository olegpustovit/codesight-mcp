//! Dead-code analysis backed by named evidence providers.

mod entrypoints;
mod reachability;

use rustc_hash::FxHashSet;

use serde_json::{Value, json};

use dowser_language_support::{
    is_barrel_basename, is_test_file, is_test_fixture, is_test_symbol, mentions_identifier_in_file,
};
use dowser_storage::storage::{AnalysisSymbol, IndexStore};

use crate::evidence::EvidenceNote;

use self::reachability::build_repo_reachability;

/// Tier 4.2: paths that typically host externally-invoked entry points the
/// reachability analyzer can't statically reach. Anything matching one of
/// these gets a confidence floor of 0.4 (below the default
/// `min_confidence=0.5`) plus a `manual_review_required` flag on the row.
///
/// Patterns are intentionally simple substring / basename checks against
/// the repo-relative path — language-agnostic so this stays in the
/// analysis crate (Rust-specific entry-file detection lives in
/// `Language::additional_live_roots`). The structural fix is the
/// per-language hook; this function is the safety net for whatever slips
/// through (e.g. `src/cli/hooks.rs` reachable only via clap subcommand
/// dispatch from `fn main`).
fn is_externally_invoked_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let has_segment = |needle: &str| -> bool { lower.split(['/', '\\']).any(|seg| seg == needle) };
    if has_segment("cli") || has_segment("bin") || has_segment("bins") {
        return true;
    }
    // Compare basenames without their extension — `.rs`, `.py`, `.go`,
    // `.ts`, etc. all use the same `main` / `lib` / `hooks` conventions
    // for entry files, and pinning to one extension would constitute a
    // per-language branch.
    let last = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    let stem = last.rsplit_once('.').map(|(s, _)| s).unwrap_or(last);
    matches!(stem, "main" | "lib" | "hook" | "hooks" | "entry")
        || stem.starts_with("hooks_")
        || stem.starts_with("hook_")
        || stem.starts_with("entry_")
}

/// Build the `data` object for `evaluate_quality` with `analysis: "dead_code"`.
///
/// `limit` caps the `dead_files` / `dead_symbols` arrays in the response
/// (full counts are always reported via `dead_file_count` / `dead_symbol_count`).
/// `None` means unbounded, preserving existing callers that pass through the
/// internal `repo_health` → `dead_code` integration.
///
/// When `paranoid=true`:
/// - the effective `min_confidence` floor is dropped to `0.0` so weak
///   signals (`weak_string_dispatch_match`, `test_only_alive`) surface;
/// - symbol-level dead detection runs regardless of `granularity`;
/// - `heuristic_disagreements` is emitted listing files the legacy
///   import-graph reachability would have flagged dead but the current
///   string-literal-dispatch heuristic rescued.
#[allow(clippy::too_many_arguments)]
pub fn dead_code_payload(
    store: &IndexStore,
    granularity: &str,
    min_confidence: f64,
    include_tests: bool,
    entry_patterns: &[String],
    limit: Option<usize>,
    include_vendored: bool,
    paranoid: bool,
) -> anyhow::Result<Value> {
    let effective_min_confidence = if paranoid { 0.0 } else { min_confidence };
    let symbol_pass = paranoid || granularity == "symbol";
    let all_symbols = store.all_symbols_for_analysis()?;
    let dep_graph = store.dependency_graph_strict()?;
    let all_files = store.all_file_paths()?;
    let root_path = store.root_path().ok();
    let file_contents = store.all_file_contents().unwrap_or_default();

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
    let files_with_code: FxHashSet<&str> = all_symbols
        .iter()
        .filter(|s| is_executable(&s.kind))
        .map(|s| s.file_path.as_str())
        .collect();

    let reachability = build_repo_reachability(
        store,
        &all_files,
        &dep_graph,
        root_path.as_deref(),
        include_tests,
        entry_patterns,
        &all_symbols,
        &file_contents,
    );

    // Filter shared across both the `dead_files` pass and the
    // `heuristic_disagreements` pass — only files that meaningfully sit
    // on the reachability graph deserve a verdict.
    let in_scope = |file: &str| -> bool {
        if !include_vendored && dowser_language_support::is_vendored_path(file) {
            return false;
        }
        if is_test_fixture(file) {
            return false;
        }
        if !include_tests && is_test_file(file) {
            return false;
        }
        if !files_with_code.contains(file) {
            return false;
        }
        true
    };

    let mut dead_files: Vec<Value> = Vec::new();
    let mut new_dead_set: FxHashSet<&str> = FxHashSet::default();
    for file in &all_files {
        if !in_scope(file.as_str()) {
            continue;
        }
        if reachability.reachable_files.contains(file) {
            continue;
        }
        // Any strong structural inbound edge (use, pub use, mod, match
        // dispatch, path reference, stored import) means *something*
        // concretely points at this file. Don't flag it. The base
        // reachability traversal already covers the import edges; this
        // catches cases where the augmentation passes added a back-edge
        // but the live-root traversal didn't happen to reach the
        // referrer in this run (e.g. cycles within an unreached
        // sub-island).
        if reachability.strong_inbound.contains(file) {
            continue;
        }

        // Test-only alive: file is reachable only through workspace test
        // binaries. Surface it separately at low confidence so callers
        // can distinguish it from prod-dead code.
        let test_only = reachability.test_only_alive.contains(file);
        let weakly_rooted = reachability.weak_string_dispatch.contains(file);

        let base_confidence: f64 = if dep_graph
            .get(file)
            .is_some_and(|targets| !targets.is_empty())
        {
            0.7
        } else {
            0.9
        };
        // Cap to 0.3 when only weak / test-only evidence supports the
        // file being alive — keeps the default `min_confidence >= 0.5`
        // filter from surfacing them while still letting probes find
        // them.
        let mut confidence = if weakly_rooted || test_only {
            base_confidence.min(0.3)
        } else {
            base_confidence
        };
        // Tier 4.2: defence-in-depth confidence floor for paths that
        // typically host externally-invoked entry points the analyzer
        // can't statically reach (CLI subcommands, hook binaries,
        // additional `[[bin]]` targets, library facades). The structural
        // fix lives in `additional_live_roots` per language; this floor
        // catches whatever falls through. Cap at 0.4 — below the default
        // `min_confidence=0.5`, so these files don't show up unless the
        // caller explicitly drops the threshold.
        let externally_invoked = is_externally_invoked_path(file);
        if externally_invoked {
            confidence = confidence.min(0.4);
        }
        if confidence < effective_min_confidence {
            continue;
        }

        let signal = if test_only {
            "test_only_alive"
        } else if weakly_rooted {
            "weak_string_dispatch_match"
        } else {
            "unreachable_from_live_roots"
        };
        let mut row = json!({
            "file": file,
            "confidence": confidence,
            "signal": signal,
        });
        if externally_invoked && let Value::Object(obj) = &mut row {
            obj.insert("manual_review_required".to_string(), json!(true));
            obj.insert(
                "manual_review_reason".to_string(),
                json!("file path matches an entry-point pattern (cli/, bin/, main.rs, hooks*.rs, lib.rs); externally-invoked code may not be statically reachable from `fn main`."),
            );
        }
        dead_files.push(row);
        new_dead_set.insert(file.as_str());
    }

    // Paranoid mode: surface files where the legacy import-graph
    // reachability disagreed with the current heuristic (i.e. files that
    // would have been dead before the string-literal-dispatch promotion
    // rescued them).
    let mut heuristic_disagreements: Vec<Value> = Vec::new();
    if paranoid {
        for file in &all_files {
            if !in_scope(file.as_str()) {
                continue;
            }
            if new_dead_set.contains(file.as_str()) {
                continue;
            }
            if reachability.legacy_reachable_files.contains(file) {
                continue;
            }
            if reachability.strong_inbound.contains(file) {
                continue;
            }
            let strong = reachability.string_dispatch_strong_roots.contains(file);
            let weak = reachability.weak_string_dispatch.contains(file);
            if !(strong || weak) {
                continue;
            }
            heuristic_disagreements.push(json!({
                "file": file,
                "old_dead": true,
                "new_dead": false,
                "new_signal": "string_literal_dispatch",
                "strong_or_weak": if strong { "strong" } else { "weak" },
            }));
        }
    }

    let mut dead_symbols: Vec<Value> = Vec::new();
    if symbol_pass {
        let dead_file_set: FxHashSet<&str> = dead_files
            .iter()
            .filter_map(|dead_file| dead_file["file"].as_str())
            .collect();
        // Symbol kinds that are meaningful targets for dead-symbol-in-alive-file
        // detection. Constants / imports / fields are too noisy.
        let is_callable_kind = |kind: &str| {
            matches!(
                kind.to_ascii_lowercase().as_str(),
                "function" | "method" | "class" | "struct" | "trait" | "enum" | "type"
            )
        };

        // Pre-compute the set of names mentioned in *some other* symbol's
        // source text in a single SQL pass. Replaces an O(N) per-candidate
        // `symbol_reference_count` call inside the loop, which made the
        // symbol-level pass quadratic over the symbol corpus and timed out
        // on a few-thousand-symbol repo.
        let candidates: Vec<(String, String)> = all_symbols
            .iter()
            .filter(|s| {
                is_callable_kind(&s.kind)
                    && s.name.len() >= 4
                    && !is_test_fixture(&s.file_path)
                    && (include_tests || !is_test_symbol(&s.file_path, &s.id))
                    && (include_vendored
                        || !dowser_language_support::is_vendored_path(&s.file_path))
            })
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let referenced_names = store
            .names_mentioned_outside_self(&candidates)
            .unwrap_or_default();

        for symbol in &all_symbols {
            if !include_vendored && dowser_language_support::is_vendored_path(&symbol.file_path) {
                continue;
            }
            if is_test_fixture(&symbol.file_path) {
                continue;
            }
            if !include_tests && is_test_symbol(&symbol.file_path, &symbol.id) {
                continue;
            }
            if dead_file_set.contains(symbol.file_path.as_str()) {
                dead_symbols.push(json!({
                    "symbol_id": symbol.id,
                    "name": symbol.name,
                    "kind": symbol.kind,
                    "file": symbol.file_path,
                    "line": symbol.start_line,
                    "confidence": 0.8,
                    "signal": "dead_file",
                }));
                continue;
            }
            // Symbol-level: name is not referenced by any structural edge.
            // Skip too-short names (collide too freely) and non-callable
            // kinds. Skip symbols inside tests files because the symbol
            // is meant to be entry-only.
            if !is_callable_kind(&symbol.kind) {
                continue;
            }
            if symbol.name.len() < 4 {
                continue;
            }
            if reachability.strong_inbound_symbols.contains(&symbol.name) {
                continue;
            }
            // Belt-and-suspenders: also accept the symbol as alive if any
            // *other* indexed symbol mentions it in source text. This
            // catches direct calls that don't pass through structural
            // edges (e.g. `foo()` invocation inside a function body).
            if referenced_names.contains(&symbol.name) {
                continue;
            }
            // Public-API guard: exported barrel members are alive.
            // A barrel basename containing the name is presumptive evidence.
            // Symbol-level "no inbound + name not mentioned anywhere" is
            // weaker evidence than file-level reachability; cap at 0.4.
            let confidence: f64 = if paranoid { 0.4 } else { 0.55 };
            if confidence < effective_min_confidence {
                continue;
            }
            dead_symbols.push(json!({
                "symbol_id": symbol.id,
                "name": symbol.name,
                "kind": symbol.kind,
                "file": symbol.file_path,
                "line": symbol.start_line,
                "confidence": confidence,
                "signal": "no_structural_inbound",
            }));
        }
    }

    let dead_file_count = dead_files.len();
    let dead_symbol_count = dead_symbols.len();
    let disagreement_count = heuristic_disagreements.len();
    let truncated = match limit {
        Some(cap) => {
            let need = dead_files.len() > cap
                || dead_symbols.len() > cap
                || heuristic_disagreements.len() > cap;
            dead_files.truncate(cap);
            dead_symbols.truncate(cap);
            heuristic_disagreements.truncate(cap);
            need
        }
        None => false,
    };
    let dead_symbols_out = if symbol_pass {
        json!(dead_symbols)
    } else {
        json!([])
    };
    let dead_symbol_count_out = if symbol_pass { dead_symbol_count } else { 0 };
    // Tier 4.2: caller-visible count of files that landed in dead_files with
    // a `manual_review_required` flag. The engine handler translates a
    // non-zero count into a top-level `MANUAL_REVIEW_REQUIRED` warning so
    // human reviewers don't auto-act on these.
    let manual_review_required_count = dead_files
        .iter()
        .filter(|f| f.get("manual_review_required").and_then(Value::as_bool) == Some(true))
        .count();
    let mut data = json!({
        "analysis": "dead_code",
        "dead_files": dead_files,
        "dead_file_count": dead_file_count,
        "dead_symbols": dead_symbols_out,
        "dead_symbol_count": dead_symbol_count_out,
        "live_root_count": reachability.live_roots.len(),
        "truncated": truncated,
        "manual_review_required_count": manual_review_required_count,
        "analysis_notes": "dead code detection uses dependency-graph reachability plus labeled entry-root heuristics; cli/bin/main/hooks paths are capped at confidence < 0.5 with a manual_review_required flag.",
    });

    if paranoid {
        data["paranoid"] = json!(true);
        data["heuristic_disagreements"] = json!(heuristic_disagreements);
        data["heuristic_disagreement_count"] = json!(disagreement_count);
    }

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
        store,
        &all_files,
        &dep_graph,
        root_path.as_deref(),
        false,
        &[],
        &all_symbols,
        &all_file_contents,
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
                "symbol name was not found in any package barrel file (__init__.py, index.ts, mod.rs, …)",
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
        if file == &symbol.file_path {
            return false;
        }
        let basename = file.rsplit('/').next().unwrap_or(file);
        is_barrel_basename(basename) && mentions_identifier_in_file(file, content, &symbol.name)
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
    let byte_budget =
        (max_tokens as usize).saturating_mul(dowser_storage::config::BYTES_PER_TOKEN as usize);
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
