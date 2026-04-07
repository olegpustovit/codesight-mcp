//! Path-based routing for extractors that are not pure extension routing.
//!
//! OpenAPI specs are selected by **basename**; Ansible by conventional **directory segments**.
//! Used by [`super::ExtractorRegistry::extractor_for_path`] and YAML/Ansible shaping in
//! [`super::yaml`].

use std::path::{Component, Path};

/// Extractor chosen before compound-extension and extension fallback rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecialExtractor {
    OpenApi,
    Ansible,
}

pub(crate) fn classify_special_path(path: &Path) -> Option<SpecialExtractor> {
    if path_suggests_openapi_basename(path) {
        return Some(SpecialExtractor::OpenApi);
    }
    if is_yamlish_extension(path) && path_suggests_ansible_layout(path) {
        return Some(SpecialExtractor::Ansible);
    }
    None
}

fn is_yamlish_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let el = e.to_ascii_lowercase();
        el == "yaml" || el == "yml"
    })
}

fn path_suggests_openapi_basename(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let stem_lower = stem.to_ascii_lowercase();
    let stem_ok = stem_lower == "openapi"
        || stem_lower == "swagger"
        || stem_lower.ends_with(".openapi")
        || stem_lower.ends_with(".swagger");
    if !stem_ok {
        return false;
    }
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let el = e.to_ascii_lowercase();
        el == "json" || el == "yaml" || el == "yml"
    })
}

/// True when any path component matches a conventional Ansible content-directory name.
pub(crate) fn path_suggests_ansible_layout(path: &Path) -> bool {
    for c in path.components() {
        let Component::Normal(part) = c else {
            continue;
        };
        let Some(s) = part.to_str() else {
            continue;
        };
        let lower = s.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "playbooks" | "roles" | "tasks" | "handlers" | "group_vars" | "host_vars"
        ) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn openapi_basenames() {
        for p in [
            "openapi.yaml",
            "swagger.yml",
            "api.openapi.json",
            "service.swagger.yaml",
        ] {
            assert_eq!(
                classify_special_path(Path::new(p)),
                Some(SpecialExtractor::OpenApi),
                "{p}"
            );
        }
    }

    #[test]
    fn openapi_negative_wrong_ext() {
        assert!(classify_special_path(Path::new("openapi.txt")).is_none());
    }

    #[test]
    fn ansible_segments_yaml() {
        for p in [
            "playbooks/site.yml",
            "roles/web/tasks/main.yaml",
            "inventory/group_vars/all.yml",
            "handlers/main.yml",
        ] {
            assert_eq!(
                classify_special_path(Path::new(p)),
                Some(SpecialExtractor::Ansible),
                "{p}"
            );
        }
    }

    #[test]
    fn generic_yaml_not_ansible() {
        assert_eq!(
            classify_special_path(Path::new("config/settings.yml")),
            None
        );
    }

    #[test]
    fn openapi_wins_over_ansible_path() {
        assert_eq!(
            classify_special_path(Path::new("playbooks/openapi.yaml")),
            Some(SpecialExtractor::OpenApi)
        );
    }
}
