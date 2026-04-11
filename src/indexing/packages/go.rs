//! Go manifest parsing: `go.mod`.

use std::fs;
use std::path::Path;

/// Read the `module` declaration from `go.mod`.
pub fn read_package_names(root: &Path) -> Vec<String> {
    let go_mod = root.join("go.mod");
    if go_mod.is_file()
        && let Ok(content) = fs::read_to_string(&go_mod)
        && let Some(name) = parse_go_mod_module(&content)
    {
        return vec![name];
    }
    Vec::new()
}

fn parse_go_mod_module(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(module_path) = trimmed.strip_prefix("module ") {
            let module_path = module_path.trim();
            if !module_path.is_empty() {
                return Some(module_path.to_string());
            }
        }
    }
    None
}

/// Extract the root Go module from an import specifier.
///
/// Domain-rooted paths such as `github.com/org/repo/subpkg` collapse to
/// `github.com/org/repo`; short module paths are returned as-is.
pub fn root_package_from_specifier(specifier: &str) -> String {
    let parts: Vec<&str> = specifier.split('/').collect();
    if parts.len() <= 3 {
        if parts[0].contains('.') && parts.len() > 3 {
            parts[..3].join("/")
        } else {
            specifier.to_string()
        }
    } else if parts[0].contains('.') {
        parts[..3].join("/")
    } else {
        specifier.to_string()
    }
}
