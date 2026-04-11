//! Rust manifest parsing: `Cargo.toml`.

use std::fs;
use std::path::Path;

/// Read the `[package] name` field from `Cargo.toml`.
pub fn read_package_names(root: &Path) -> Vec<String> {
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.is_file()
        && let Ok(content) = fs::read_to_string(&cargo_toml)
        && let Some(name) = parse_cargo_toml_name(&content)
    {
        return vec![name];
    }
    Vec::new()
}

fn parse_cargo_toml_name(content: &str) -> Option<String> {
    let table: toml::Value = content.parse().ok()?;
    table
        .get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

/// Extract the crate name from a Rust path specifier (`serde::de::T` → `serde`).
pub fn root_package_from_specifier(specifier: &str) -> String {
    specifier.split("::").next().unwrap_or("").to_string()
}
