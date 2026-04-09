use std::path::Path;

use crate::framework::{FileContext, FrameworkProvider};

use super::project::detect_dbt_project;
use super::render::{metadata, render_file_context};
use super::snapshot::DbtSnapshot;

/// Stats for the dbt provider.
#[derive(Debug, Clone, Default)]
pub struct DbtStats {
    pub doc_blocks: usize,
    pub models_with_metadata: usize,
}

pub struct DbtContextProvider {
    snapshot: Option<DbtSnapshot>,
}

impl DbtContextProvider {
    pub fn new() -> Self {
        Self { snapshot: None }
    }

    pub fn stats(&self) -> DbtStats {
        self.snapshot
            .as_ref()
            .map(DbtSnapshot::stats)
            .unwrap_or_default()
    }
}

impl Default for DbtContextProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for DbtContextProvider {
    fn name(&self) -> &str {
        "dbt"
    }

    fn detect(&self, root: &Path) -> bool {
        detect_dbt_project(root).is_some()
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.snapshot = Some(DbtSnapshot::load(root)?);
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
}
