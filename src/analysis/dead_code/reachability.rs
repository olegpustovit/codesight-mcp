use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use crate::{analysis::evidence::EvidenceNote, storage::AnalysisSymbol};

use super::entrypoints::{
    detect_cargo_target_files, detect_main_guard_files, entrypoint_root_notes, is_test_file,
};

pub(crate) struct RepoReachability {
    pub live_roots: HashSet<String>,
    pub explicit_live_roots: HashSet<String>,
    pub reachable_files: HashSet<String>,
    pub evidence_notes: Vec<EvidenceNote>,
    pub framework_warning: Option<String>,
}

pub(crate) fn build_repo_reachability(
    all_files: &[String],
    dep_graph: &HashMap<String, Vec<String>>,
    all_symbols: &[AnalysisSymbol],
    root_path: Option<&Path>,
    include_tests: bool,
    entry_patterns: &[String],
) -> RepoReachability {
    let main_guard_files = detect_main_guard_files(root_path, all_files, all_symbols);
    let cargo_target_files = detect_cargo_target_files(root_path, all_files);
    let mut live_roots = HashSet::new();
    let mut explicit_live_roots = HashSet::new();
    let mut evidence_notes = Vec::new();
    let mut explicit_roots_found = false;

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

        if main_guard_files.contains(file) {
            live_roots.insert(file.clone());
            explicit_live_roots.insert(file.clone());
            evidence_notes.push(EvidenceNote::heuristic(
                "main_guard",
                format!("file '{file}' contains a __name__ == '__main__' style guard"),
            ));
        }

        if cargo_target_files.contains(file) {
            explicit_roots_found = true;
            live_roots.insert(file.clone());
            explicit_live_roots.insert(file.clone());
            evidence_notes.push(EvidenceNote::structural(
                "cargo_target",
                format!("file '{file}' is a Cargo [[test]]/[[example]]/[[bench]]/[[bin]] target"),
            ));
        }
    }

    let active_graph = filtered_graph(dep_graph, include_tests);
    let reachable_files = traverse_reachable_files(&active_graph, &live_roots);
    let framework_warning = if explicit_roots_found || !main_guard_files.is_empty() {
        None
    } else {
        Some(
            "No standard entry points detected (e.g. main.py, app.py). Dead code results may be inaccurate without known entry points. Consider specifying entry_point_patterns.".to_string(),
        )
    };

    RepoReachability {
        live_roots,
        explicit_live_roots,
        reachable_files,
        evidence_notes,
        framework_warning,
    }
}

fn filtered_graph(
    dep_graph: &HashMap<String, Vec<String>>,
    include_tests: bool,
) -> HashMap<String, Vec<String>> {
    dep_graph
        .iter()
        .filter(|(source, _)| include_tests || !is_test_file(source))
        .map(|(source, targets)| {
            let filtered_targets = targets
                .iter()
                .filter(|target| include_tests || !is_test_file(target))
                .cloned()
                .collect();
            (source.clone(), filtered_targets)
        })
        .collect()
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
