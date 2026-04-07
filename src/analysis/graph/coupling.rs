//! Afferent / efferent coupling metrics from an adjacency list.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CouplingMetrics {
    pub module: String,
    /// Afferent coupling: number of files that import this file
    pub ca: usize,
    /// Efferent coupling: number of files this file imports
    pub ce: usize,
    /// Instability: Ce / (Ca + Ce), or None if both are 0
    pub instability: Option<f64>,
    pub assessment: String,
    pub importers: Vec<String>,
    pub dependencies: Vec<String>,
}

pub fn compute_coupling(file_path: &str, adj: &HashMap<String, Vec<String>>) -> CouplingMetrics {
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

    let assessment = match instability {
        Some(i) if i <= 0.3 => "stable".to_string(),
        Some(i) if i >= 0.7 => "unstable".to_string(),
        Some(_) => "balanced".to_string(),
        None => "isolated".to_string(),
    };

    CouplingMetrics {
        module: file_path.to_string(),
        ca,
        ce,
        instability,
        assessment,
        importers,
        dependencies,
    }
}
