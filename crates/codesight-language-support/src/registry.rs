//! `LanguageRegistry` — the single entry point every crate above L0.75
//! uses to look up language identity and behavior.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use crate::languages::register_builtins;
use crate::manifest::{HostProfile, LanguageId, ProfileId, ResolvedLanguage, UNKNOWN_LANGUAGE};
use crate::traits::Language;

pub struct LanguageRegistry {
    by_id: HashMap<LanguageId, Arc<dyn Language>>,
    by_filename: HashMap<String, LanguageId>,
    by_compound_ext: HashMap<String, LanguageId>,
    by_ext: HashMap<String, LanguageId>,
    extra_ext: HashMap<String, LanguageId>,
    profiles: Vec<(LanguageId, &'static HostProfile)>,
    curated: BTreeSet<LanguageId>,
}

impl LanguageRegistry {
    /// Empty registry. Useful for tests that want to assert zero-state
    /// behavior. Production callers use [`LanguageRegistry::builtin`] or
    /// [`LanguageRegistry::for_workspace_root`].
    pub fn empty() -> Self {
        Self {
            by_id: HashMap::new(),
            by_filename: HashMap::new(),
            by_compound_ext: HashMap::new(),
            by_ext: HashMap::new(),
            extra_ext: HashMap::new(),
            profiles: Vec::new(),
            curated: BTreeSet::new(),
        }
    }

    /// Registry populated from every builtin language.
    pub fn builtin() -> Self {
        let mut reg = Self::empty();
        register_builtins(&mut reg);
        reg
    }

    /// Registry populated from builtin languages plus workspace-local overlays
    /// and `.codesight.toml` `extra_extensions`.
    pub fn for_workspace_root(root: &Path) -> anyhow::Result<Self> {
        let mut reg = Self::builtin();
        reg.register_workspace_query_packs(root)?;
        let extras = crate::workspace::project_extra_extensions(root);
        let typed: HashMap<String, LanguageId> = extras
            .iter()
            .map(|(ext, lang_name)| {
                let id = reg
                    .all()
                    .find(|lang| {
                        let name = lang.id().as_str();
                        if name.eq_ignore_ascii_case(lang_name) {
                            return true;
                        }
                        lang.manifest()
                            .aliases
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(lang_name))
                    })
                    .map(|lang| lang.id())
                    .unwrap_or(UNKNOWN_LANGUAGE);
                (ext.clone(), id)
            })
            .collect();
        reg.apply_extra_extension_ids(&typed);
        Ok(reg)
    }

    fn register_workspace_query_packs(&mut self, root: &Path) -> anyhow::Result<()> {
        let overlay_roots = crate::workspace::project_query_pack_roots(root);
        let overlay = crate::query_pack_catalog::QueryPackCatalog::from_roots(&overlay_roots)?;
        for pack in overlay.iter() {
            let id = LanguageId::intern(&pack.metadata.name);
            if self.by_id.contains_key(&id) {
                continue;
            }
            let Some(spec) = crate::grammars::builtin_grammar_spec(&pack.metadata.grammar) else {
                continue;
            };
            let lang = crate::workspace_pack::WorkspacePackLanguage::new(id, pack, spec);
            self.register(Arc::new(lang));
        }
        Ok(())
    }

    pub fn get(&self, id: LanguageId) -> Option<&dyn Language> {
        self.by_id.get(&id).map(Arc::as_ref)
    }

    pub fn for_path(&self, path: &Path) -> Option<&dyn Language> {
        let host = self.identify_path(path);
        if let Some(profile_lang) = self
            .best_profile_for(path, host)
            .and_then(|pid| self.for_language_name(pid.as_str()))
        {
            return Some(profile_lang);
        }
        self.resolve_get(host)
    }

    /// Legacy alias for [`Self::for_path`].
    pub fn extractor_for_path(&self, path: &Path) -> Option<&dyn Language> {
        self.for_path(path)
    }

    /// Legacy alias for [`Self::for_language_name`].
    pub fn extractor_for_language(&self, name: &str) -> Option<&dyn Language> {
        self.for_language_name(name)
    }

    /// Legacy helper: every supported language as a sorted list of names.
    pub fn supported_languages(&self) -> Vec<&str> {
        let mut langs: Vec<&str> = self.all().map(|lang| lang.id().as_str()).collect();
        langs.sort();
        langs.dedup();
        langs
    }

    /// Legacy alias for [`Self::supported_languages`].
    pub fn known_languages(&self) -> Vec<&str> {
        self.supported_languages()
    }

    /// Curated-language names as a sorted list (string form).
    pub fn curated_language_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> =
            self.curated_languages().map(|id| id.as_str()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// Resolve a language name passed by the caller (e.g. `"rust"`,
    /// `"golang"`, `"tsx"`). Looks up by canonical id OR by manifest alias.
    pub fn for_language_name(&self, name: &str) -> Option<&dyn Language> {
        let lower = name.to_ascii_lowercase();
        self.all().find(|lang| {
            if lang.id().as_str().eq_ignore_ascii_case(&lower) {
                return true;
            }
            lang.manifest()
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&lower))
        })
    }

    pub fn for_file(&self, path: &Path, source: Option<&str>) -> Option<&dyn Language> {
        let id = match source {
            Some(src) => self.identify_with_source(path, src),
            None => self.identify_path(path),
        };
        self.resolve_get(id)
    }

    /// Host-language identity from `path` alone.
    pub fn identify_path(&self, path: &Path) -> LanguageId {
        self.identify_path_no_probe(path)
    }

    /// Source-aware identity including the content-probe tie-breaker.
    pub fn identify_with_source(&self, path: &Path, source: &str) -> LanguageId {
        let host = self.identify_path_no_probe(path);
        if host == UNKNOWN_LANGUAGE {
            return host;
        }
        if let Some(probe) = self
            .by_id
            .get(&host)
            .and_then(|lang| lang.manifest().content_probe)
            && let Some(refined) = probe(path, source)
            && self.by_id.contains_key(&refined)
        {
            return refined;
        }
        host
    }

    pub fn classify_path(&self, path: &Path) -> ResolvedLanguage {
        let host = self.identify_path(path);
        let profile = self.best_profile_for(path, host);
        match profile {
            Some(id) => ResolvedLanguage::with_profile(host, id),
            None => ResolvedLanguage::host_only(host),
        }
    }

    pub fn classify_with_source(&self, path: &Path, source: &str) -> ResolvedLanguage {
        let host = self.identify_with_source(path, source);
        let profile = self.best_profile_for(path, host);
        match profile {
            Some(id) => ResolvedLanguage::with_profile(host, id),
            None => ResolvedLanguage::host_only(host),
        }
    }

    pub fn all(&self) -> impl Iterator<Item = &dyn Language> {
        self.by_id.values().map(Arc::as_ref)
    }

    pub fn curated_languages(&self) -> impl Iterator<Item = LanguageId> + '_ {
        self.curated.iter().copied()
    }

    pub fn mark_curated(&mut self, id: LanguageId) {
        if self.by_id.contains_key(&id) {
            self.curated.insert(id);
        }
    }

    pub fn register(&mut self, language: Arc<dyn Language>) {
        let id = language.id();
        let manifest = language.manifest();
        for name in manifest.filename_overrides {
            self.by_filename.insert(name.to_ascii_lowercase(), id);
        }
        for ext in manifest.compound_extensions {
            self.by_compound_ext.insert(ext.to_ascii_lowercase(), id);
        }
        for ext in manifest.extensions {
            self.by_ext
                .entry(ext.to_ascii_lowercase())
                .or_insert(id);
        }
        for profile in manifest.profiles {
            for host in profile.applies_to {
                self.profiles.push((*host, profile));
            }
        }
        self.by_id.insert(id, language);
    }

    /// Id-keyed register: accepts pre-resolved `(extension, LanguageId)`
    /// pairs. Used by internal call-sites that already have a typed id.
    /// External callers / `.codesight.toml` use [`Self::apply_extra_extensions`].
    pub fn apply_extra_extension_ids(
        &mut self,
        mappings: &HashMap<String, LanguageId>,
    ) -> Vec<String> {
        let mut rejected = Vec::new();
        for (ext, id) in mappings {
            let trimmed = ext.trim_start_matches('.').to_ascii_lowercase();
            if trimmed.is_empty() || !self.by_id.contains_key(id) {
                rejected.push(ext.clone());
                continue;
            }
            self.extra_ext.insert(trimmed, *id);
        }
        rejected
    }

    /// Register extra `(extension, language_name)` mappings. Returns a list
    /// of human-readable warnings for entries the registry could not accept
    /// (empty extension, unknown language, etc.).
    pub fn apply_extra_extensions(
        &mut self,
        mappings: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        for (ext_raw, lang_raw) in mappings {
            let ext = ext_raw.trim().to_string();
            let lang = lang_raw.trim().to_string();
            if ext.is_empty() || ext == "." {
                warnings.push(format!(
                    "extra_extensions: skipping empty extension for language '{lang_raw}'"
                ));
                continue;
            }
            if lang.is_empty() {
                warnings.push(format!(
                    "extra_extensions: skipping extension '{ext}' with empty language"
                ));
                continue;
            }
            let ext_norm = ext.strip_prefix('.').unwrap_or(&ext).to_ascii_lowercase();
            let Some(lang_ref) = self.for_language_name(&lang) else {
                warnings.push(format!(
                    "extra_extensions: unknown language '{lang}' for extension '.{ext_norm}', skipping"
                ));
                continue;
            };
            self.extra_ext.insert(ext_norm, lang_ref.id());
        }
        warnings
    }

    fn identify_path_no_probe(&self, path: &Path) -> LanguageId {
        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
            let lower = file_name.to_ascii_lowercase();
            if let Some(id) = self.by_filename.get(&lower) {
                return *id;
            }
            for (compound, id) in &self.by_compound_ext {
                let suffix = format!(".{compound}");
                if lower.ends_with(&suffix) && lower.len() > suffix.len() {
                    return *id;
                }
            }
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_ascii_lowercase(),
            None => return UNKNOWN_LANGUAGE,
        };
        if let Some(id) = self.extra_ext.get(&ext) {
            return *id;
        }
        self.by_ext.get(&ext).copied().unwrap_or(UNKNOWN_LANGUAGE)
    }

    fn best_profile_for(&self, path: &Path, host: LanguageId) -> Option<ProfileId> {
        if host == UNKNOWN_LANGUAGE {
            return None;
        }
        let mut best: Option<&HostProfile> = None;
        for (applies, profile) in &self.profiles {
            if *applies != host {
                continue;
            }
            if !(profile.matches_path)(path) {
                continue;
            }
            best = match best {
                Some(cur) if cur.priority <= profile.priority => Some(cur),
                _ => Some(profile),
            };
        }
        best.map(|p| p.id)
    }

    fn resolve_get(&self, id: LanguageId) -> Option<&dyn Language> {
        if id == UNKNOWN_LANGUAGE {
            None
        } else {
            self.get(id)
        }
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BLADE, C, CPP, JSON, MARKDOWN, PYTHON, RUST, TSX, TYPESCRIPT, YAML};

    fn reg() -> LanguageRegistry {
        LanguageRegistry::builtin()
    }

    #[test]
    fn identify_common_extensions() {
        let r = reg();
        assert_eq!(r.identify_path(Path::new("src/main.rs")), RUST);
        assert_eq!(r.identify_path(Path::new("a.ts")), TYPESCRIPT);
        assert_eq!(r.identify_path(Path::new("a.tsx")), TSX);
        assert_eq!(r.identify_path(Path::new("a.py")), PYTHON);
        assert_eq!(r.identify_path(Path::new("data.json")), JSON);
        assert_eq!(r.identify_path(Path::new("README.md")), MARKDOWN);
    }

    #[test]
    fn compound_extension_wins_over_single() {
        let r = reg();
        assert_eq!(r.identify_path(Path::new("views/layout.blade.php")), BLADE);
        assert_eq!(r.identify_path(Path::new("types.d.ts")), TYPESCRIPT);
    }

    #[test]
    fn h_header_defaults_to_cpp_without_source() {
        assert_eq!(reg().identify_path(Path::new("foo.h")), CPP);
    }

    #[test]
    fn h_header_content_probe_downgrades_to_c() {
        let r = reg();
        let plain_c = "#ifndef FOO\n#define FOO\nvoid f(void);\n#endif\n";
        assert_eq!(r.identify_with_source(Path::new("foo.h"), plain_c), C);
    }

    #[test]
    fn h_header_content_probe_keeps_cpp_for_namespaced_code() {
        let r = reg();
        let cpp = "namespace n { void f(); }\n";
        assert_eq!(r.identify_with_source(Path::new("foo.h"), cpp), CPP);
    }

    #[test]
    fn openapi_profile_attaches_to_yaml() {
        let r = reg();
        let resolved = r.classify_path(Path::new("api/openapi.yaml"));
        assert_eq!(resolved.host, YAML);
        assert_eq!(resolved.profile.map(|p| p.as_str()), Some("openapi"));
    }

    #[test]
    fn openapi_profile_attaches_to_json() {
        let r = reg();
        let resolved = r.classify_path(Path::new("api.openapi.json"));
        assert_eq!(resolved.host, JSON);
        assert_eq!(resolved.profile.map(|p| p.as_str()), Some("openapi"));
    }

    #[test]
    fn openapi_beats_ansible_on_contested_yaml() {
        let r = reg();
        let resolved = r.classify_path(Path::new("playbooks/openapi.yaml"));
        assert_eq!(resolved.host, YAML);
        assert_eq!(resolved.profile.map(|p| p.as_str()), Some("openapi"));
    }

    #[test]
    fn ansible_profile_matches_conventional_layout() {
        let r = reg();
        let resolved = r.classify_path(Path::new("roles/web/tasks/main.yml"));
        assert_eq!(resolved.host, YAML);
        assert_eq!(resolved.profile.map(|p| p.as_str()), Some("ansible"));
    }

    #[test]
    fn unknown_extension_returns_unknown_language() {
        let r = reg();
        assert_eq!(r.identify_path(Path::new("a.xyznope")), UNKNOWN_LANGUAGE);
        assert!(r.for_path(Path::new("a.xyznope")).is_none());
    }

    #[test]
    fn apply_extra_extensions_rejects_unknown_language_id() {
        let mut r = LanguageRegistry::empty();
        let mut mappings = HashMap::new();
        mappings.insert(".foo".to_string(), RUST);
        let rejected = r.apply_extra_extension_ids(&mappings);
        assert_eq!(rejected, vec![".foo".to_string()]);
    }

    #[test]
    fn apply_extra_extensions_routes_registered_extension() {
        let mut r = LanguageRegistry::builtin();
        let mut mappings = HashMap::new();
        mappings.insert(".rsx".to_string(), RUST);
        assert!(r.apply_extra_extension_ids(&mappings).is_empty());
        assert_eq!(r.identify_path(Path::new("foo.rsx")), RUST);
    }

    #[test]
    fn curated_languages_includes_core_four() {
        let r = reg();
        let curated: Vec<_> = r.curated_languages().collect();
        assert!(curated.contains(&RUST));
        assert!(curated.contains(&PYTHON));
        assert!(curated.contains(&TYPESCRIPT));
        assert!(curated.contains(&YAML));
    }

    #[test]
    fn builtin_registry_registers_no_unknown_entries() {
        let r = reg();
        assert!(r.all().count() >= 30);
        assert!(r.get(UNKNOWN_LANGUAGE).is_none());
    }

    #[test]
    fn phase_c_every_curated_manifest_has_grammar_and_tags() {
        let r = reg();
        let mut checked = 0usize;
        for lang in r.all() {
            if let Some(grammar) = lang.manifest().grammar {
                assert!(
                    !grammar.tags_scm.is_empty(),
                    "{} grammar has empty tags_scm",
                    lang.id()
                );
                assert!(
                    crate::grammars::builtin_grammar(grammar.grammar_name).is_some(),
                    "{} grammar_name {:?} is not resolvable",
                    lang.id(),
                    grammar.grammar_name
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 25,
            "expected ≥ 25 languages to carry a grammar, got {checked}"
        );
    }

    #[test]
    fn phase_c_curated_set_matches_query_pack_directories() {
        let r = reg();
        let resolved = |pack_id: &str| -> bool {
            r.all().any(|lang| {
                lang.id().as_str() == pack_id
                    || lang
                        .manifest()
                        .aliases
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(pack_id))
            })
        };
        for (pack_id, _, _, _) in crate::grammars::builtin_query_packs() {
            assert!(resolved(pack_id), "query pack {pack_id:?} unreachable");
        }
    }

    #[test]
    fn phase_c_for_workspace_root_applies_extra_extensions_idempotently() {
        use std::fs;

        let tmp = std::env::temp_dir().join("codesight-phase-c-overlay");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(
            tmp.join(".codesight.toml"),
            "[extra_extensions]\n\"ron\" = \"rust\"\n",
        )
        .unwrap();

        let a = LanguageRegistry::for_workspace_root(&tmp).unwrap();
        let b = LanguageRegistry::for_workspace_root(&tmp).unwrap();
        assert_eq!(a.identify_path(Path::new("tmp/x.ron")), RUST);
        assert_eq!(b.identify_path(Path::new("tmp/x.ron")), RUST);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn phase_c_workspace_config_rejects_unknown_keys() {
        use crate::workspace::read_project_config;
        use std::fs;

        let tmp = std::env::temp_dir().join("codesight-phase-c-bad-config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(
            tmp.join(".codesight.toml"),
            "[nonsense]\nnope = true\n",
        )
        .unwrap();
        let cfg = read_project_config(&tmp);
        assert!(cfg.is_some());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn phase_c_extra_extensions_rejects_malformed_entries() {
        let mut r = LanguageRegistry::builtin();
        let mut mappings = HashMap::new();
        mappings.insert(String::new(), RUST);
        mappings.insert(".".to_string(), RUST);
        mappings.insert(".xx".to_string(), LanguageId::new("nosuchlanguage"));
        let rejected = r.apply_extra_extension_ids(&mappings);
        assert_eq!(rejected.len(), 3);
        assert_eq!(r.identify_path(Path::new("foo.xx")), UNKNOWN_LANGUAGE);
    }

    #[test]
    fn phase_c_injection_targets_resolve_to_registered_languages() {
        let r = reg();
        for lang in r.all() {
            let Some(grammar) = lang.manifest().grammar else {
                continue;
            };
            let Some(injections) = grammar.injections_scm else {
                continue;
            };
            for line in injections.lines() {
                if line.trim_start().starts_with(';') {
                    continue;
                }
                let Some(idx) = line.find("injection.language") else {
                    continue;
                };
                let rest = &line[idx + "injection.language".len()..];
                let quote_start = rest
                    .char_indices()
                    .find(|(_, c)| *c == '"')
                    .map(|(i, _)| i);
                let Some(start) = quote_start else { continue };
                let after = &rest[start + 1..];
                let Some(end) = after.find('"') else { continue };
                let target = &after[..end];
                let resolved = r.all().any(|entry| {
                    let manifest = entry.manifest();
                    entry.id().as_str().eq_ignore_ascii_case(target)
                        || manifest
                            .aliases
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(target))
                        || manifest
                            .grammar
                            .map(|g| g.grammar_name.eq_ignore_ascii_case(target))
                            .unwrap_or(false)
                });
                assert!(
                    resolved,
                    "{} injections.scm references unknown language {:?}",
                    lang.id(),
                    target
                );
            }
        }
    }
}
