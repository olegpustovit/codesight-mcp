//! Git-derived file churn and dependency-graph fan-in — both inputs to the
//! importance weighting used across every submetric.

use rustc_hash::FxHashMap;
use std::path::Path;

pub fn compute_fan_in(adj: &FxHashMap<String, Vec<String>>) -> FxHashMap<String, usize> {
    let mut fan_in: FxHashMap<String, usize> = FxHashMap::default();
    for src in adj.keys() {
        fan_in.entry(src.clone()).or_insert(0);
    }
    for targets in adj.values() {
        for tgt in targets {
            *fan_in.entry(tgt.clone()).or_insert(0) += 1;
        }
    }
    fan_in
}

pub fn compute_file_churn(root: &Path) -> FxHashMap<String, u64> {
    let mut churn = FxHashMap::default();
    if !root.join(".git").exists() {
        return churn;
    }
    let Ok(output) = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only"])
        .current_dir(root)
        .output()
    else {
        return churn;
    };
    if !output.status.success() {
        return churn;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            *churn.entry(trimmed.to_string()).or_default() += 1;
        }
    }
    churn
}
