//! Package manifest discovery, registry, and cross-repo edge materialization.
//! Owned by the indexing domain (index-time package metadata).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Extract package names from manifest files in a directory.
///
/// Searches for `pyproject.toml`, `setup.cfg`, `package.json`, `go.mod`,
/// `Cargo.toml`, and `*.csproj` to discover declared package names.
pub fn extract_package_names(dir: &str) -> Vec<String> {
    let root = Path::new(dir);
    if !root.is_dir() {
        return vec![];
    }

    let mut names = Vec::new();

    // Python: pyproject.toml takes precedence over setup.cfg
    let pyproject = root.join("pyproject.toml");
    let setup_cfg = root.join("setup.cfg");
    let mut found_python = false;

    if pyproject.is_file()
        && let Ok(content) = fs::read_to_string(&pyproject)
        && let Some(name) = parse_pyproject_name(&content)
    {
        // PEP 503 normalization: underscores -> hyphens
        names.push(name.replace('_', "-"));
        found_python = true;
    }

    if !found_python
        && setup_cfg.is_file()
        && let Ok(content) = fs::read_to_string(&setup_cfg)
        && let Some(name) = parse_setup_cfg_name(&content)
    {
        names.push(name);
    }

    // JavaScript: package.json
    let package_json = root.join("package.json");
    if package_json.is_file()
        && let Ok(content) = fs::read_to_string(&package_json)
        && let Some(name) = parse_package_json_name(&content)
    {
        names.push(name);
    }

    // Go: go.mod
    let go_mod = root.join("go.mod");
    if go_mod.is_file()
        && let Ok(content) = fs::read_to_string(&go_mod)
        && let Some(name) = parse_go_mod_module(&content)
    {
        names.push(name);
    }

    // Rust: Cargo.toml
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.is_file()
        && let Ok(content) = fs::read_to_string(&cargo_toml)
        && let Some(name) = parse_cargo_toml_name(&content)
    {
        names.push(name);
    }

    // C#: *.csproj
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

fn parse_pyproject_name(content: &str) -> Option<String> {
    // Simple TOML parsing for [project] name = "..."
    let table: toml::Value = content.parse().ok()?;
    table
        .get("project")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

fn parse_setup_cfg_name(content: &str) -> Option<String> {
    // Simple ini-style parsing for [metadata] name = ...
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

fn parse_package_json_name(content: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    v.get("name")?.as_str().map(|s| s.to_string())
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

fn parse_cargo_toml_name(content: &str) -> Option<String> {
    let table: toml::Value = content.parse().ok()?;
    table
        .get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

fn parse_csproj_name(content: &str) -> Option<String> {
    // Simple XML element extraction for <PackageName> or <AssemblyName>
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

/// Extract the root package name from an import specifier for a given language.
///
/// Returns empty string for relative imports.
pub fn extract_root_package_from_specifier(specifier: &str, language: &str) -> String {
    let lang = language.to_ascii_lowercase();
    match lang.as_str() {
        "python" => {
            if specifier.starts_with('.') {
                return String::new();
            }
            specifier.split('.').next().unwrap_or("").to_string()
        }
        "javascript" | "typescript" | "tsx" | "jsx" => {
            if specifier.starts_with('.') {
                return String::new();
            }
            if specifier.starts_with('@') {
                // Scoped package: @org/package/sub -> @org/package
                let parts: Vec<&str> = specifier.splitn(3, '/').collect();
                if parts.len() >= 2 {
                    return format!("{}/{}", parts[0], parts[1]);
                }
                return specifier.to_string();
            }
            // Unscoped: lodash/merge -> lodash
            specifier.split('/').next().unwrap_or("").to_string()
        }
        "go" => {
            // Go modules: github.com/org/repo/pkg -> github.com/org/repo (first 3 segments)
            let parts: Vec<&str> = specifier.split('/').collect();
            if parts.len() <= 3 {
                // Short module or exactly 3 segments
                if parts[0].contains('.') && parts.len() > 3 {
                    // Domain-based path, take first 3
                    parts[..3].join("/")
                } else {
                    specifier.to_string()
                }
            } else if parts[0].contains('.') {
                // github.com/org/repo/subpkg -> github.com/org/repo
                parts[..3].join("/")
            } else {
                specifier.to_string()
            }
        }
        "rust" => {
            // serde::de::Deserialize -> serde
            specifier.split("::").next().unwrap_or("").to_string()
        }
        _ => specifier.to_string(),
    }
}

/// A mapping from package name -> list of repo IDs that publish it.
pub type PackageRegistry = HashMap<String, Vec<String>>;

/// Repo entry for building the package registry.
#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub repo_id: String,
    pub package_names: Vec<String>,
}

/// Build a registry mapping package names to the repo IDs that publish them.
pub fn build_package_registry(repos: &[RepoEntry]) -> PackageRegistry {
    let mut registry = PackageRegistry::new();
    for entry in repos {
        for name in &entry.package_names {
            registry
                .entry(name.clone())
                .or_default()
                .push(entry.repo_id.clone());
        }
    }
    registry
}

/// Find repo IDs that publish a given package name.
pub fn find_repos_for_package(package: &str, repos: &[RepoEntry]) -> Vec<String> {
    let registry = build_package_registry(repos);
    registry.get(package).cloned().unwrap_or_default()
}

/// Cross-repo edge in the dependency map.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossRepoEdge {
    pub from_repo: String,
    pub to_repo: String,
    pub package_name: String,
}

/// Build a cross-repo dependency map for all indexed repos.
///
/// For each repo, look at its imported packages, find which other repos
/// publish those packages, and create edges.
pub fn build_cross_repo_edges(
    repos: &[RepoEntry],
    repo_imports: &HashMap<String, Vec<String>>,
) -> Vec<CrossRepoEdge> {
    let registry = build_package_registry(repos);
    let mut edges = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (consumer_repo, import_specifiers) in repo_imports {
        for specifier in import_specifiers {
            // Try to figure out the language from the specifier
            // We'll try each language's root extraction
            let root_candidates: Vec<String> = vec![
                extract_root_package_from_specifier(specifier, "python"),
                extract_root_package_from_specifier(specifier, "javascript"),
                extract_root_package_from_specifier(specifier, "go"),
                extract_root_package_from_specifier(specifier, "rust"),
            ];

            for root_pkg in &root_candidates {
                if root_pkg.is_empty() {
                    continue;
                }
                if let Some(provider_repos) = registry.get(root_pkg) {
                    for provider_repo in provider_repos {
                        if provider_repo == consumer_repo {
                            continue; // Skip self-references
                        }
                        let key = (
                            consumer_repo.clone(),
                            provider_repo.clone(),
                            root_pkg.clone(),
                        );
                        if seen.insert(key) {
                            edges.push(CrossRepoEdge {
                                from_repo: consumer_repo.clone(),
                                to_repo: provider_repo.clone(),
                                package_name: root_pkg.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_package_names ──

    #[test]
    fn extract_package_names_python_pyproject() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"my-lib\"\n",
        )
        .unwrap();
        let names = extract_package_names(dir.path().to_str().unwrap());
        assert!(names.contains(&"my-lib".to_string()));
    }

    #[test]
    fn extract_package_names_python_underscore_normalization() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"my_lib\"\n",
        )
        .unwrap();
        let names = extract_package_names(dir.path().to_str().unwrap());
        assert!(names.contains(&"my-lib".to_string()));
    }

    #[test]
    fn extract_package_names_nonexistent_dir() {
        let names = extract_package_names("/nonexistent/path/that/does/not/exist");
        assert!(names.is_empty());
    }

    // ── extract_root_package_from_specifier ──

    #[test]
    fn root_package_python_simple() {
        assert_eq!(
            extract_root_package_from_specifier("flask", "python"),
            "flask"
        );
    }

    #[test]
    fn root_package_python_dotted() {
        assert_eq!(
            extract_root_package_from_specifier("flask.blueprints", "python"),
            "flask"
        );
    }

    #[test]
    fn root_package_python_relative() {
        assert_eq!(extract_root_package_from_specifier(".utils", "python"), "");
    }

    #[test]
    fn root_package_js_scoped() {
        assert_eq!(
            extract_root_package_from_specifier("@org/package", "javascript"),
            "@org/package"
        );
    }

    #[test]
    fn root_package_js_scoped_with_subpath() {
        assert_eq!(
            extract_root_package_from_specifier("@org/package/utils", "javascript"),
            "@org/package"
        );
    }

    #[test]
    fn root_package_rust_crate_path() {
        assert_eq!(
            extract_root_package_from_specifier("serde::de::Deserialize", "rust"),
            "serde"
        );
    }

    // ── build_package_registry ──

    #[test]
    fn registry_maps_package_to_repo() {
        let repos = vec![RepoEntry {
            repo_id: "local/repo-a".to_string(),
            package_names: vec!["requests".to_string()],
        }];
        let registry = build_package_registry(&repos);
        assert!(registry["requests"].contains(&"local/repo-a".to_string()));
    }

    #[test]
    fn find_repos_nonexistent_package() {
        let repos = vec![RepoEntry {
            repo_id: "local/repo-a".to_string(),
            package_names: vec!["numpy".to_string()],
        }];
        let result = find_repos_for_package("pandas", &repos);
        assert!(result.is_empty());
    }
}
