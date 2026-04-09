use std::path::Path;

use crate::framework::{ExtraImport, FileContext, FrameworkProvider};

use super::render::{metadata, render_file_context};
use super::snapshot::NuxtSnapshot;

#[derive(Debug, Clone, Default)]
pub struct NuxtStats {
    pub page_routes: usize,
    pub api_routes: usize,
    pub auto_import_symbols: usize,
}

pub struct NuxtProvider {
    snapshot: Option<NuxtSnapshot>,
}

impl NuxtProvider {
    pub fn new() -> Self {
        Self { snapshot: None }
    }

    pub fn stats(&self) -> NuxtStats {
        self.snapshot
            .as_ref()
            .map(NuxtSnapshot::stats)
            .unwrap_or_default()
    }
}

impl Default for NuxtProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for NuxtProvider {
    fn name(&self) -> &str {
        "nuxt"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("nuxt.config.ts").exists() || root.join("nuxt.config.js").exists()
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.snapshot = Some(NuxtSnapshot::load(root));
        Ok(())
    }

    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| render_file_context(snapshot, rel_path))
    }

    fn get_metadata(&self) -> std::collections::HashMap<String, serde_json::Value> {
        self.snapshot.as_ref().map(metadata).unwrap_or_default()
    }

    fn get_extra_imports(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<ExtraImport>>> {
        Ok(self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.extra_imports.clone())
            .unwrap_or_default())
    }
}
