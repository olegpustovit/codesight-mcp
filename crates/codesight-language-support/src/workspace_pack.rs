//! Synthetic `Language` for workspace-defined query packs.
//!
//! When a project ships a custom SCM pack under `.codesight/queries/<lang>/`
//! or a `query_pack_roots`-listed directory, `LanguageRegistry::for_workspace_root`
//! builds a `WorkspacePackLanguage` so the custom language flows through the
//! same trait surface as every builtin. The grammar loader is reused from an
//! existing builtin grammar (matched by `pack.toml`'s `grammar` field); SCM
//! and manifest metadata are leaked to `&'static` at registration — the
//! workspace pack set is small and bounded.

use codesight_core::StoredSymbol;

use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest};
use crate::query_pack_catalog::QueryPackDefinition;
use crate::traits::Language;

pub struct WorkspacePackLanguage {
    id: LanguageId,
    manifest: &'static LanguageManifest,
    spec: &'static GrammarSpec,
}

impl WorkspacePackLanguage {
    pub fn new(
        id: LanguageId,
        pack: &QueryPackDefinition,
        builtin_spec: &'static GrammarSpec,
    ) -> Self {
        let extensions: &'static [&'static str] = Box::leak(
            pack.metadata
                .extensions
                .iter()
                .map(|ext| &*Box::leak(ext.clone().into_boxed_str()))
                .collect::<Vec<&'static str>>()
                .into_boxed_slice(),
        );
        let display_name: &'static str = Box::leak(pack.metadata.name.clone().into_boxed_str());
        let manifest: &'static LanguageManifest = Box::leak(Box::new(LanguageManifest {
            display_name,
            aliases: &[],
            extensions,
            filename_overrides: &[],
            compound_extensions: &[],
            grammar: Some(builtin_spec),
            profiles: &[],
            content_probe: None,
        }));
        let tags_scm: &'static str = Box::leak(pack.query_source.clone().into_boxed_str());
        let injections_scm: Option<&'static str> = pack
            .injections_source
            .as_ref()
            .map(|src| &*Box::leak(src.clone().into_boxed_str()));
        let spec: &'static GrammarSpec = Box::leak(Box::new(GrammarSpec {
            grammar_name: builtin_spec.grammar_name,
            loader: builtin_spec.loader,
            tags_scm,
            injections_scm,
        }));
        Self { id, manifest, spec }
    }
}

impl Language for WorkspacePackLanguage {
    fn id(&self) -> LanguageId {
        self.id
    }
    fn manifest(&self) -> &'static LanguageManifest {
        self.manifest
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(self.spec)
    }
    fn supports_extraction(&self) -> bool {
        true
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        crate::scm_engine::extract_with_spec(self.id.as_str(), self.spec, file_path, source)
    }
}
