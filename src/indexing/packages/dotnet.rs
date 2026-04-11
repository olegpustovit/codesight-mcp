//! .NET manifest parsing: `*.csproj`.

use std::fs;
use std::path::Path;

/// Read `<PackageName>` or `<AssemblyName>` from each `*.csproj` in `root`.
pub fn read_package_names(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("csproj")
                && let Ok(content) = fs::read_to_string(&path)
                && let Some(name) = parse_csproj_name(&content)
            {
                names.push(name);
            }
        }
    }
    names
}

fn parse_csproj_name(content: &str) -> Option<String> {
    if let Some(name) = extract_xml_element(content, "PackageName") {
        return Some(name);
    }
    extract_xml_element(content, "AssemblyName")
}

fn extract_xml_element(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    if let Some(start) = content.find(&open) {
        let after_open = start + open.len();
        if let Some(end) = content[after_open..].find(&close) {
            let value = content[after_open..after_open + end].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
