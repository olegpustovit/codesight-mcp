//! Layer definitions, path-to-layer mapping, and import boundary violations.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDef {
    pub name: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub may_not_import: Vec<String>,
}

/// Map a file path to its layer name based on prefix matching.
/// Matches on path segment boundaries (won't match `src/apiother` against `src/api`).
pub fn file_to_layer(path: &str, layers: &[LayerDef]) -> Option<String> {
    for layer in layers {
        for prefix in &layer.paths {
            // Exact match
            if path == prefix {
                return Some(layer.name.clone());
            }
            // Prefix match with path separator boundary
            let with_sep = if prefix.ends_with('/') {
                prefix.clone()
            } else {
                format!("{prefix}/")
            };
            if path.starts_with(&with_sep) {
                return Some(layer.name.clone());
            }
            // Also match if the filename itself matches the prefix (for flat repos)
            // e.g., prefix="routes.py" matches file "routes.py"
            if let Some(file_name) = path.rsplit('/').next() {
                if file_name == prefix {
                    return Some(layer.name.clone());
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerViolation {
    pub file: String,
    pub file_layer: String,
    pub import_target: String,
    pub target_layer: String,
    pub rule_violated: String,
}

pub fn find_layer_violations(
    adj: &HashMap<String, Vec<String>>,
    rules: &[LayerDef],
) -> Vec<LayerViolation> {
    let mut violations = Vec::new();

    // Build a quick lookup: layer_name → may_not_import set
    let prohibited: HashMap<&str, HashSet<&str>> = rules
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.may_not_import.iter().map(|s| s.as_str()).collect(),
            )
        })
        .collect();

    for (file, deps) in adj {
        let Some(src_layer) = file_to_layer(file, rules) else {
            continue;
        };
        let Some(blocked) = prohibited.get(src_layer.as_str()) else {
            continue;
        };

        for dep in deps {
            let Some(dep_layer) = file_to_layer(dep, rules) else {
                continue;
            };
            if blocked.contains(dep_layer.as_str()) {
                violations.push(LayerViolation {
                    file: file.clone(),
                    file_layer: src_layer.clone(),
                    import_target: dep.clone(),
                    target_layer: dep_layer.clone(),
                    rule_violated: format!("{src_layer} may_not_import {dep_layer}"),
                });
            }
        }
    }

    violations.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.import_target.cmp(&b.import_target))
    });
    violations
}
