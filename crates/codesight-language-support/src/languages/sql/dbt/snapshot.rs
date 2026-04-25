use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

use crate::frameworks::walk_files;

use super::docs::{parse_doc_blocks, parse_doc_blocks_from_content};
use super::project::detect_dbt_project;
use super::schema::{DbtModelMetadata, parse_yml_content, parse_yml_files};

/// Stats for the dbt provider.
#[derive(Debug, Clone, Default)]
pub struct DbtStats {
    pub doc_blocks: usize,
    pub models_with_metadata: usize,
}

#[derive(Debug, Clone)]
pub struct DbtSnapshot {
    pub(crate) dbt_root: PathBuf,
    pub(crate) model_paths: Vec<String>,
    pub(crate) doc_blocks: FxHashMap<String, String>,
    pub(crate) models: FxHashMap<String, DbtModelMetadata>,
}

impl DbtSnapshot {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let dbt_root = detect_dbt_project(root).ok_or_else(|| anyhow::anyhow!("no dbt project"))?;

        let mut snapshot = Self {
            dbt_root,
            model_paths: vec!["models".into()],
            doc_blocks: FxHashMap::default(),
            models: FxHashMap::default(),
        };
        snapshot.load_model_paths();
        snapshot.load_doc_blocks();
        snapshot.load_models();
        Ok(snapshot)
    }

    pub fn stats(&self) -> DbtStats {
        DbtStats {
            doc_blocks: self.doc_blocks.len(),
            models_with_metadata: self.models.len(),
        }
    }

    pub(crate) fn is_in_model_dir(&self, rel_path: &str) -> bool {
        let normalized = rel_path.replace('\\', "/");
        self.model_paths.iter().any(|model_path| {
            let prefix = if model_path.ends_with('/') {
                model_path.clone()
            } else {
                format!("{model_path}/")
            };
            normalized.starts_with(&prefix)
        })
    }

    pub(crate) fn file_stem(rel_path: &str) -> Option<String> {
        let normalized = rel_path.replace('\\', "/");
        Path::new(&normalized)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.to_string())
    }

    fn load_model_paths(&mut self) {
        let project_yml = self.dbt_root.join("dbt_project.yml");
        if let Ok(content) = std::fs::read_to_string(project_yml)
            && let Ok(yaml) = serde_yml::from_str::<serde_yml::Value>(&content)
            && let Some(serde_yml::Value::Sequence(paths)) = yaml.get("model-paths")
        {
            self.model_paths = paths
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect();
        }
    }

    fn load_doc_blocks(&mut self) {
        let model_paths = self.model_paths.clone();
        for model_path in &model_paths {
            let model_dir = self.dbt_root.join(model_path);
            if model_dir.exists() {
                self.collect_doc_blocks_under(&model_dir);
            }
        }

        let docs_dir = self.dbt_root.join("docs");
        if docs_dir.exists() {
            self.doc_blocks.extend(parse_doc_blocks(&docs_dir));
        }
    }

    fn load_models(&mut self) {
        let doc_blocks = self.doc_blocks.clone();
        let model_paths = self.model_paths.clone();
        for model_path in &model_paths {
            let model_dir = self.dbt_root.join(model_path);
            if !model_dir.exists() {
                continue;
            }

            self.models.extend(parse_yml_files(&model_dir, &doc_blocks));
            self.collect_yml_with_blocks(&model_dir, &doc_blocks);
        }
    }

    fn collect_doc_blocks_under(&mut self, dir: &Path) {
        for path in walk_files(dir) {
            if path.extension().is_some_and(|ext| ext == "md")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                self.doc_blocks
                    .extend(parse_doc_blocks_from_content(&content));
            }
        }
    }

    fn collect_yml_with_blocks(&mut self, dir: &Path, doc_blocks: &FxHashMap<String, String>) {
        for path in walk_files(dir) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if (ext == "yml" || ext == "yaml")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                self.models.extend(parse_yml_content(&content, doc_blocks));
            }
        }
    }
}
