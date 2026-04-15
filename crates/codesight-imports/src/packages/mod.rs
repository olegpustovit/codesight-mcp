//! Package manifest discovery, registry, and cross-repo edge materialization.
//!
//! Owned by the indexing domain (index-time package metadata). Per-ecosystem
//! manifest parsing lives in the submodules; the [`walker`] composes them
//! into a monorepo-aware discovery pass; the [`registry`] consumes the
//! resulting package facts for cross-repo edge building.

use std::path::Path;

pub mod dotnet;
pub mod go;
pub mod javascript;
pub mod python;
pub mod registry;
pub mod rust;
pub mod walker;

pub use registry::{
    CrossRepoEdge, PackageRegistry, RepoEntry, build_cross_repo_edges, build_package_registry,
    find_repos_for_package,
};
pub use walker::discover_package_names;

/// Extract package names from every supported manifest discovered under
/// `dir`, walking nested workspace members in monorepo layouts.
///
/// Supported manifests: `pyproject.toml`, `setup.cfg`, `package.json`,
/// `go.mod`, `Cargo.toml`, and `*.csproj`. Build outputs, dependencies,
/// virtualenvs, VCS metadata, and dotted directories are pruned during
/// discovery. See [`walker`] for the full pruning policy.
pub fn extract_package_names(dir: &str) -> Vec<String> {
    discover_package_names(Path::new(dir))
}

/// Extract the root package name from an import specifier for a given language.
///
/// Returns empty string for relative imports. Languages not explicitly
/// supported fall through to returning the specifier unchanged.
pub fn extract_root_package_from_specifier(specifier: &str, language: &str) -> String {
    match language.to_ascii_lowercase().as_str() {
        "python" => python::root_package_from_specifier(specifier),
        "javascript" | "typescript" | "tsx" | "jsx" => {
            javascript::root_package_from_specifier(specifier)
        }
        "go" => go::root_package_from_specifier(specifier),
        "rust" => rust::root_package_from_specifier(specifier),
        _ => specifier.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
