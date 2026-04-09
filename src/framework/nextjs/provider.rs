use std::path::Path;

use crate::framework::{FileContext, FrameworkProvider};

use super::render::{metadata, render_file_context};
use super::snapshot::NextjsSnapshot;

#[derive(Debug, Clone, Default)]
pub struct NextjsStats {
    pub page_routes: usize,
    pub api_routes: usize,
}

pub struct NextjsProvider {
    snapshot: Option<NextjsSnapshot>,
}

impl NextjsProvider {
    pub fn new() -> Self {
        Self { snapshot: None }
    }

    pub fn stats(&self) -> NextjsStats {
        self.snapshot
            .as_ref()
            .map(NextjsSnapshot::stats)
            .unwrap_or_default()
    }
}

impl Default for NextjsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameworkProvider for NextjsProvider {
    fn name(&self) -> &str {
        "nextjs"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join("next.config.js").exists()
            || root.join("next.config.mjs").exists()
            || root.join("next.config.ts").exists()
    }

    fn load(&mut self, root: &Path) -> anyhow::Result<()> {
        self.snapshot = Some(NextjsSnapshot::load(root));
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
