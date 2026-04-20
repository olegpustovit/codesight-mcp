//! YAML host + OpenAPI / Ansible host profiles (LANGUAGES.md §3.7, §6.2).
//!
//! Phase B moves the classifier from
//! `codesight-extractors::extractors::classifier` into a first-class host
//! profile. OpenAPI wins over Ansible (priority 0 < priority 10) and also
//! attaches to the JSON host via `applies_to`.

use std::path::{Component, Path};

use crate::manifest::{HostProfile, JSON, ProfileId, YAML};

pub const OPENAPI: ProfileId = ProfileId::new("openapi");
pub const ANSIBLE: ProfileId = ProfileId::new("ansible");

pub static OPENAPI_PROFILE: HostProfile = HostProfile {
    id: OPENAPI,
    priority: 0,
    matches_path: matches_openapi_path,
    applies_to: &[YAML, JSON],
};

pub static ANSIBLE_PROFILE: HostProfile = HostProfile {
    id: ANSIBLE,
    priority: 10,
    matches_path: matches_ansible_path,
    applies_to: &[YAML],
};

pub static YAML_PROFILES: &[HostProfile] = &[OPENAPI_PROFILE, ANSIBLE_PROFILE];
pub static JSON_PROFILES: &[HostProfile] = &[OPENAPI_PROFILE];

fn matches_openapi_path(path: &Path) -> bool {
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

fn matches_ansible_path(path: &Path) -> bool {
    let yamlish = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let el = e.to_ascii_lowercase();
        el == "yaml" || el == "yml"
    });
    if !yamlish {
        return false;
    }
    for c in path.components() {
        let Component::Normal(part) = c else { continue };
        let Some(s) = part.to_str() else { continue };
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

    #[test]
    fn openapi_matches_yaml_and_json() {
        for p in [
            "openapi.yaml",
            "swagger.yml",
            "api.openapi.json",
            "service.swagger.yaml",
        ] {
            assert!(matches_openapi_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn openapi_rejects_wrong_extension() {
        assert!(!matches_openapi_path(Path::new("openapi.txt")));
    }

    #[test]
    fn ansible_matches_conventional_layouts() {
        for p in [
            "playbooks/site.yml",
            "roles/web/tasks/main.yaml",
            "inventory/group_vars/all.yml",
            "handlers/main.yml",
        ] {
            assert!(matches_ansible_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn ansible_ignores_non_yaml() {
        assert!(!matches_ansible_path(Path::new("playbooks/site.json")));
    }

    #[test]
    fn generic_yaml_is_not_ansible() {
        assert!(!matches_ansible_path(Path::new("config/settings.yml")));
    }
}
