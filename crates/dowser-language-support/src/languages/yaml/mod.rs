//! YAML host + OpenAPI / Ansible host profiles.

pub mod extractor;

use std::path::{Component, Path};

use anyhow::Result;
use dowser_core::StoredSymbol;
use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::YAML_MANIFEST;
use crate::manifest::{
    GrammarSpec, HostProfile, JSON, LanguageId, LanguageManifest, ProfileId, YAML,
};
use crate::traits::Language;

pub struct YamlSupport;

fn load() -> TsLanguage {
    tree_sitter_yaml::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "yaml",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for YamlSupport {
    fn id(&self) -> LanguageId {
        YAML
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &YAML_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        extractor::YamlExtractor.extract(file_path, source)
    }
    fn is_reference_target(&self) -> bool {
        false
    }
}

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
