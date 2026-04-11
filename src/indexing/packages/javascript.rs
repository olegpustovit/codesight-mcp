//! JavaScript / TypeScript manifest parsing: `package.json`.

use std::fs;
use std::path::Path;

/// Read the `name` field from `package.json`, if present.
pub fn read_package_names(root: &Path) -> Vec<String> {
    let package_json = root.join("package.json");
    if package_json.is_file()
        && let Ok(content) = fs::read_to_string(&package_json)
        && let Some(name) = parse_package_json_name(&content)
    {
        return vec![name];
    }
    Vec::new()
}

fn parse_package_json_name(content: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    v.get("name")?.as_str().map(|s| s.to_string())
}

/// Extract the root npm package from an import specifier.
///
/// Returns empty string for relative imports. Scoped packages (`@org/pkg`)
/// keep both segments; unscoped packages (`lodash/merge`) collapse to the
/// first segment.
pub fn root_package_from_specifier(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    if specifier.starts_with('@') {
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return format!("{}/{}", parts[0], parts[1]);
        }
        return specifier.to_string();
    }
    specifier.split('/').next().unwrap_or("").to_string()
}
