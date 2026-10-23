//! Ansible — host-profile variant of YAML. Matched by
//! `playbooks/*.yml`, `roles/*`, `tasks/*`, `handlers/*`, `group_vars/*`,
//! `host_vars/*` paths. Inherits YAML's grammar; the YAML extractor
//! already branches internally on Ansible paths to emit play / task /
//! handler symbols.

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::ANSIBLE_MANIFEST;
use crate::languages::yaml;
use crate::manifest::{ANSIBLE, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::Language;
use symslice_core::StoredSymbol;
use symslice_core::symbols::ExtractionTier;

pub struct Ansible;

impl Language for Ansible {
    fn id(&self) -> LanguageId {
        ANSIBLE
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &ANSIBLE_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&yaml::GRAMMAR)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::ManualOverride
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        yaml::extractor::YamlExtractor.extract(file_path, source)
    }
}
