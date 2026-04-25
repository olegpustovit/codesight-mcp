//! Afferent / efferent coupling metrics from an adjacency list.

use rustc_hash::FxHashMap;

use serde::Serialize;

use crate::thresholds::current_thresholds;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CouplingAssessment {
    /// Instability ≤ `thresholds.coupling_stable_max`.
    Stable,
    /// Between the stable ceiling and the unstable floor — neither extreme.
    Balanced,
    /// Instability ≥ `thresholds.coupling_unstable_min`.
    Unstable,
    /// No afferent or efferent coupling.
    Isolated,
}

impl CouplingAssessment {
    pub fn classify(instability: Option<f64>) -> Self {
        let thresholds = current_thresholds();
        match instability {
            Some(i) if i <= thresholds.coupling_stable_max => Self::Stable,
            Some(i) if i >= thresholds.coupling_unstable_min => Self::Unstable,
            Some(_) => Self::Balanced,
            None => Self::Isolated,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CouplingMetrics {
    pub module: String,
    /// Afferent coupling: number of files that import this file.
    pub ca: usize,
    /// Efferent coupling: number of files this file imports.
    pub ce: usize,
    /// Instability: `Ce / (Ca + Ce)`, or `None` when both are zero.
    pub instability: Option<f64>,
    pub assessment: CouplingAssessment,
    pub importers: Vec<String>,
    pub dependencies: Vec<String>,
}

pub fn compute_coupling(file_path: &str, adj: &FxHashMap<String, Vec<String>>) -> CouplingMetrics {
    let dependencies: Vec<String> = adj.get(file_path).cloned().unwrap_or_default();
    let ce = dependencies.len();

    let mut importers = Vec::new();
    for (source, deps) in adj {
        if source != file_path && deps.contains(&file_path.to_string()) {
            importers.push(source.clone());
        }
    }
    importers.sort();
    let ca = importers.len();

    let instability = if ca + ce > 0 {
        Some(ce as f64 / (ca + ce) as f64)
    } else {
        None
    };

    CouplingMetrics {
        module: file_path.to_string(),
        ca,
        ce,
        instability,
        assessment: CouplingAssessment::classify(instability),
        importers,
        dependencies,
    }
}
