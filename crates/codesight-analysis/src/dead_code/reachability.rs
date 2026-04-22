use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use codesight_language_support::is_test_file;
use codesight_language_support::traits::LiveRootConfidence;

use crate::evidence::EvidenceNote;

use super::entrypoints::{detect_language_live_roots, entrypoint_root_notes};

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
    root_path: Option<&Path>,
    include_tests: bool,
    entry_patterns: &[String],
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

    let active_graph = filtered_graph(dep_graph, include_tests);
    let reachable_files = traverse_reachable_files(&active_graph, &live_roots);
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
