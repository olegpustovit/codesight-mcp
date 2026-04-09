use std::path::Path;

use crate::framework::{ExtraImport, FileContext, FrameworkProvider};

use super::render::{metadata, render_file_context};
use super::snapshot::LaravelSnapshot;

#[derive(Debug, Clone, Default)]
pub struct LaravelStats {
    pub models: usize,
    pub migration_tables: usize,
    pub routes: usize,
    pub blade_files: usize,
    pub route_files_with_imports: usize,
    pub blade_files_with_imports: usize,
}

pub struct LaravelProvider {
    snapshot: Option<LaravelSnapshot>,
}

impl LaravelProvider {
    pub fn new() -> Self {
        Self { snapshot: None }
    }

    pub fn stats(&self) -> LaravelStats {
        self.snapshot
            .as_ref()
            .map(LaravelSnapshot::stats)
            .unwrap_or_default()
    }
}

impl Default for LaravelProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for LaravelProvider {
    fn name(&self) -> &str {
        "laravel"
    }

    fn detect(&self, root: &Path) -> bool {
        if !root.join("artisan").exists() {
            return false;
        }
        match std::fs::read_to_string(root.join("composer.json")) {
            Ok(content) => content.contains("laravel/framework"),
            Err(_) => false,
        }
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.snapshot = Some(LaravelSnapshot::load(root));
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
