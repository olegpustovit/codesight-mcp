//! Python manifest parsing: `pyproject.toml` and `setup.cfg`.

use std::fs;
use std::path::Path;

/// Read package names declared in `pyproject.toml` or `setup.cfg`.
///
/// `pyproject.toml` takes precedence. Names are PEP 503-normalized
/// (underscores become hyphens).
pub fn read_package_names(root: &Path) -> Vec<String> {
    let pyproject = root.join("pyproject.toml");
    if pyproject.is_file()
        && let Ok(content) = fs::read_to_string(&pyproject)
        && let Some(name) = parse_pyproject_name(&content)
    {
        return vec![name.replace('_', "-")];
    }

    let setup_cfg = root.join("setup.cfg");
    if setup_cfg.is_file()
        && let Ok(content) = fs::read_to_string(&setup_cfg)
        && let Some(name) = parse_setup_cfg_name(&content)
    {
        return vec![name];
    }

    Vec::new()
}

fn parse_pyproject_name(content: &str) -> Option<String> {
    let table: toml::Value = content.parse().ok()?;
    table
        .get("project")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

fn parse_setup_cfg_name(content: &str) -> Option<String> {
    let mut in_metadata = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_metadata = trimmed == "[metadata]";
            continue;
        }
        if in_metadata && let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Extract the root Python module from a dotted import specifier.
///
/// Returns empty string for relative imports (`.foo`).
pub fn root_package_from_specifier(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    specifier.split('.').next().unwrap_or("").to_string()
}
