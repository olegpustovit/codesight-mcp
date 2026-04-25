//! Repo reachability for dead-code analysis.
//!
//! Reachability edges are produced per-file at ingest time by
//! `Language::reachability_edges` and persisted in the
//! `reachability_edges` table. This module reads from that table and
//! traverses the union of (a) the import dep_graph, (b) language-emitted
//! file-target edges, plus (c) symbol-target edges resolved against the
//! workspace symbol index. No file I/O happens here on the hot path.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use codesight_core::reachability::EdgeKind;
use codesight_language_support::is_test_file;
use codesight_language_support::traits::LiveRootConfidence;
use codesight_storage::storage::{AnalysisSymbol, IndexStore};

use crate::evidence::EvidenceNote;

use super::entrypoints::{detect_language_live_roots, entrypoint_root_notes};

/// Minimum number of distinct external files that must mention a symbol
/// name as a string literal before the symbol's defining file is promoted
/// to a structural live root. One reference is weak evidence; two or more
/// lines up with genuine dispatch tables.
const STRING_DISPATCH_STRONG_REFS: usize = 2;

pub(crate) struct RepoReachability {
    pub live_roots: HashSet<String>,
    pub explicit_live_roots: HashSet<String>,
    pub reachable_files: HashSet<String>,
    /// Files where a defined symbol's name appears as a string literal in
    /// exactly one other file — weak evidence of indirect dispatch.
    pub weak_string_dispatch: HashSet<String>,
    /// Files reached only through test-binary roots.
    pub test_only_alive: HashSet<String>,
    /// Files with a strong structural inbound edge (from the persisted
    /// `reachability_edges` table — re-export, mod decl, path ref, match
    /// dispatch, import).
    pub strong_inbound: HashSet<String>,
    /// Bare symbol names that some file references via a structural edge.
    /// A symbol whose name appears here is **not** dead. Filled from
    /// `reachability_edges.target_symbol`.
    pub strong_inbound_symbols: HashSet<String>,
    pub evidence_notes: Vec<EvidenceNote>,
    pub framework_warning: Option<String>,
}

pub(crate) fn build_repo_reachability(
    store: &IndexStore,
    all_files: &[String],
    dep_graph: &HashMap<String, Vec<String>>,
    root_path: Option<&Path>,
    include_tests: bool,
    entry_patterns: &[String],
    all_symbols: &[AnalysisSymbol],
    file_contents: &[(String, String)],
) -> RepoReachability {
    let language_roots = detect_language_live_roots(root_path, all_files);
    let all_files_set: HashSet<&str> = all_files.iter().map(String::as_str).collect();

    let mut live_roots = HashSet::new();
    let mut explicit_live_roots = HashSet::new();
    let mut evidence_notes = Vec::new();
    let mut explicit_roots_found = false;
    let mut language_entry_contributed = false;

    for file in all_files {
        if !include_tests && is_test_file(file) {
            continue;
        }
        let entry_notes = entrypoint_root_notes(file, entry_patterns);
        if !entry_notes.is_empty() {
            explicit_roots_found = true;
            live_roots.insert(file.clone());
            explicit_live_roots.insert(file.clone());
            evidence_notes.extend(entry_notes);
        }
    }

    for root in language_roots {
        if !all_files_set.contains(root.path.as_str()) {
            continue;
        }
        if !include_tests && is_test_file(&root.path) {
            continue;
        }
        live_roots.insert(root.path.clone());
        if root.explicit {
            explicit_live_roots.insert(root.path.clone());
            explicit_roots_found = true;
        }
        language_entry_contributed = true;
        evidence_notes.push(match root.confidence {
            LiveRootConfidence::Structural => EvidenceNote::structural(root.reason, root.message),
            LiveRootConfidence::Heuristic => EvidenceNote::heuristic(root.reason, root.message),
        });
    }

    let StringDispatchSignals {
        strong_roots,
        weak_roots,
    } = detect_string_dispatch_signals(all_symbols, file_contents, include_tests);
    for file in &strong_roots {
        if live_roots.insert(file.clone()) {
            evidence_notes.push(EvidenceNote::heuristic(
                "string_literal_dispatch",
                format!(
                    "'{file}' defines a symbol whose name appears as a string \
                     literal in {STRING_DISPATCH_STRONG_REFS}+ other files — \
                     likely dispatched via a router / handler table / plugin \
                     registry"
                ),
            ));
        }
    }

    // Read reachability edges from storage. The five file-scan
    // augmentation passes that used to live here have moved into
    // `Language::reachability_edges` and run once per file at ingest.
    let stored_edges = store
        .reachability_edges(&[
            EdgeKind::ReExport,
            EdgeKind::PathReference,
            EdgeKind::MatchDispatch,
            EdgeKind::ModuleDecl,
            EdgeKind::ImportStatement,
        ])
        .unwrap_or_default();

    // Resolve symbol-target edges back to file targets via the symbol
    // index, and accumulate `strong_inbound` / `strong_inbound_symbols`.
    let symbol_index = build_symbol_index(all_symbols, include_tests);
    let mut extra_edges: HashMap<String, HashSet<String>> = HashMap::new();
    let mut strong_inbound: HashSet<String> = HashSet::new();
    let mut strong_inbound_symbols: HashSet<String> = HashSet::new();
    for edge in &stored_edges {
        if !include_tests && is_test_file(&edge.source_file) {
            continue;
        }
        if let Some(target_file) = &edge.target_file {
            if !include_tests && is_test_file(target_file) {
                continue;
            }
            extra_edges
                .entry(edge.source_file.clone())
                .or_default()
                .insert(target_file.clone());
            strong_inbound.insert(target_file.clone());
        }
        if let Some(target_symbol) = &edge.target_symbol {
            strong_inbound_symbols.insert(target_symbol.clone());
            if let Some(defining_files) = symbol_index.get(target_symbol.as_str()) {
                for def in defining_files {
                    if *def == edge.source_file.as_str() {
                        continue;
                    }
                    if !include_tests && is_test_file(def) {
                        continue;
                    }
                    extra_edges
                        .entry(edge.source_file.clone())
                        .or_default()
                        .insert((*def).to_string());
                    strong_inbound.insert((*def).to_string());
                }
            }
        }
    }

    let active_graph = filtered_graph_with_extra(dep_graph, &extra_edges, include_tests);
    let reachable_files = traverse_reachable_files(&active_graph, &live_roots);

    // Workspace test-target binaries: files only reached through them
    // get marked test-only-alive (so they don't appear as dead but can be
    // surfaced separately).
    let test_only_alive = if include_tests {
        HashSet::new()
    } else {
        compute_test_only_alive(all_files, dep_graph, &extra_edges, &live_roots, &reachable_files)
    };

    let framework_warning = if explicit_roots_found || language_entry_contributed {
        None
    } else {
        Some(
            "No explicit entry points detected. Dead-code results may be inaccurate \
             without known entry points — configure `entry_point_patterns` or add a \
             language that declares live roots."
                .to_string(),
        )
    };

    RepoReachability {
        live_roots,
        explicit_live_roots,
        reachable_files,
        weak_string_dispatch: weak_roots,
        test_only_alive,
        strong_inbound,
        strong_inbound_symbols,
        evidence_notes,
        framework_warning,
    }
}

/// `name -> set of files defining a symbol with that name`. Limited to
/// callable / type kinds; constants and value bindings are excluded
/// because their names collide too freely with arbitrary identifiers.
type SymbolIndex<'a> = HashMap<&'a str, HashSet<&'a str>>;

fn build_symbol_index<'a>(
    all_symbols: &'a [AnalysisSymbol],
    include_tests: bool,
) -> SymbolIndex<'a> {
    let mut idx: SymbolIndex<'a> = HashMap::new();
    let is_indexed = |kind: &str| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "function"
                | "method"
                | "class"
                | "trait"
                | "struct"
                | "enum"
                | "type"
                | "module"
                | "interface"
        )
    };
    for sym in all_symbols {
        if !is_indexed(&sym.kind) {
            continue;
        }
        if sym.name.is_empty() {
            continue;
        }
        if !include_tests && is_test_file(&sym.file_path) {
            continue;
        }
        idx.entry(sym.name.as_str())
            .or_default()
            .insert(sym.file_path.as_str());
    }
    idx
}

struct StringDispatchSignals {
    /// Files with ≥ `STRING_DISPATCH_STRONG_REFS` external string-literal
    /// mentions of a symbol they define.
    strong_roots: HashSet<String>,
    /// Files with exactly one external string-literal mention.
    weak_roots: HashSet<String>,
}

/// Scan every indexed file's raw text for string-literal tokens that match
/// a function / method / class / struct / enum / trait name defined
/// somewhere in the workspace. Returns defining files partitioned by
/// evidence strength. Handles router-like indirect dispatch across every
/// language without hard-coding specific frameworks.
fn detect_string_dispatch_signals(
    all_symbols: &[AnalysisSymbol],
    file_contents: &[(String, String)],
    include_tests: bool,
) -> StringDispatchSignals {
    let is_dispatchable = |kind: &str| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "function" | "method" | "class" | "struct" | "trait" | "enum"
        )
    };

    let mut name_to_defs: HashMap<&str, HashSet<&str>> = HashMap::new();
    for symbol in all_symbols {
        if !is_dispatchable(&symbol.kind) {
            continue;
        }
        if symbol.name.len() < 3 {
            continue;
        }
        if !include_tests && is_test_file(&symbol.file_path) {
            continue;
        }
        name_to_defs
            .entry(symbol.name.as_str())
            .or_default()
            .insert(symbol.file_path.as_str());
    }
    if name_to_defs.is_empty() {
        return StringDispatchSignals {
            strong_roots: HashSet::new(),
            weak_roots: HashSet::new(),
        };
    }

    let mut external_referrers: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (file, content) in file_contents {
        if !include_tests && is_test_file(file) {
            continue;
        }
        for token in iter_string_literal_identifiers(content) {
            if let Some(defining_files) = name_to_defs.get(token) {
                for def in defining_files {
                    if *def != file.as_str() {
                        external_referrers
                            .entry(*def)
                            .or_default()
                            .insert(file.as_str());
                    }
                }
            }
        }
    }

    let mut strong_roots = HashSet::new();
    let mut weak_roots = HashSet::new();
    for (def, referrers) in external_referrers {
        if referrers.len() >= STRING_DISPATCH_STRONG_REFS {
            strong_roots.insert(def.to_string());
        } else {
            weak_roots.insert(def.to_string());
        }
    }
    StringDispatchSignals {
        strong_roots,
        weak_roots,
    }
}

/// Yield identifier-shaped tokens that appear inside `"…"` or `'…'` string
/// literals. Deliberately naive byte-scan — false negatives are tolerable
/// (additive heuristic) and false positives are bounded by the
/// identifier-kind filter.
fn iter_string_literal_identifiers(source: &str) -> impl Iterator<Item = &str> {
    let bytes = source.as_bytes();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'\n' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() && bytes[j] == quote {
                let body = &source[start..j];
                for tok in body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                    if tok.len() >= 3
                        && tok
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    {
                        out.push(tok);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out.into_iter()
}

fn filtered_graph_with_extra(
    dep_graph: &HashMap<String, Vec<String>>,
    extra_edges: &HashMap<String, HashSet<String>>,
    include_tests: bool,
) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (source, targets) in dep_graph {
        if !include_tests && is_test_file(source) {
            continue;
        }
        let mut keep: HashSet<String> = HashSet::new();
        for target in targets {
            if include_tests || !is_test_file(target) {
                keep.insert(target.clone());
            }
        }
        out.insert(source.clone(), keep.into_iter().collect());
    }
    for (source, targets) in extra_edges {
        if !include_tests && is_test_file(source) {
            continue;
        }
        let entry = out.entry(source.clone()).or_default();
        let existing: HashSet<String> = entry.iter().cloned().collect();
        for target in targets {
            if (include_tests || !is_test_file(target)) && !existing.contains(target) {
                entry.push(target.clone());
            }
        }
    }
    out
}

fn traverse_reachable_files(
    dep_graph: &HashMap<String, Vec<String>>,
    live_roots: &HashSet<String>,
) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue: VecDeque<String> = live_roots.iter().cloned().collect();

    while let Some(file) = queue.pop_front() {
        if !reachable.insert(file.clone()) {
            continue;
        }
        if let Some(targets) = dep_graph.get(&file) {
            for target in targets {
                if !reachable.contains(target) {
                    queue.push_back(target.clone());
                }
            }
        }
    }

    reachable
}

/// Compute the set of files that are reachable only via workspace test
/// binaries (`tests/*.rs`, `crates/*/tests/**/*.rs`).
fn compute_test_only_alive(
    all_files: &[String],
    dep_graph: &HashMap<String, Vec<String>>,
    extra_edges: &HashMap<String, HashSet<String>>,
    prod_live_roots: &HashSet<String>,
    prod_reachable: &HashSet<String>,
) -> HashSet<String> {
    let mut test_seeds: HashSet<String> = HashSet::new();
    for file in all_files {
        if is_test_target_root(file) {
            test_seeds.insert(file.clone());
        }
    }
    if test_seeds.is_empty() {
        return HashSet::new();
    }
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for (s, ts) in dep_graph {
        graph.entry(s.clone()).or_default().extend(ts.iter().cloned());
    }
    for (s, ts) in extra_edges {
        graph.entry(s.clone()).or_default().extend(ts.iter().cloned());
    }
    let mut seeds = prod_live_roots.clone();
    for t in &test_seeds {
        seeds.insert(t.clone());
    }
    let test_reachable = traverse_reachable_files(&graph, &seeds);
    test_reachable.difference(prod_reachable).cloned().collect()
}

fn is_test_target_root(file: &str) -> bool {
    if !file.ends_with(".rs") {
        return false;
    }
    if file.starts_with("tests/") {
        return file[6..].matches('/').count() == 0;
    }
    if let Some(idx) = file.rfind("/tests/") {
        let rest = &file[idx + 7..];
        return !rest.contains('/');
    }
    false
}
