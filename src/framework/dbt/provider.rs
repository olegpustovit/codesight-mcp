use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::framework::{FileContext, FrameworkProvider};

use super::docs::{parse_doc_blocks, parse_doc_blocks_from_content};
use super::project::detect_dbt_project;
use super::schema::{DbtModelMetadata, parse_yml_content, parse_yml_files};

/// Stats for the dbt provider.
#[derive(Debug, Clone, Default)]
pub struct DbtStats {
    pub doc_blocks: usize,
    pub models_with_metadata: usize,
}

pub struct DbtContextProvider {
    dbt_root: Option<PathBuf>,
    model_paths: Vec<String>,
    doc_blocks: HashMap<String, String>,
    models: HashMap<String, DbtModelMetadata>,
}

impl DbtContextProvider {
    pub fn new() -> Self {
        Self {
            dbt_root: None,
            model_paths: vec!["models".into()],
            doc_blocks: HashMap::new(),
            models: HashMap::new(),
        }
    }

    pub fn stats(&self) -> DbtStats {
        DbtStats {
            doc_blocks: self.doc_blocks.len(),
            models_with_metadata: self.models.len(),
        }
    }

    /// Check whether `rel_path` is inside one of the model directories.
    fn is_in_model_dir(&self, rel_path: &str) -> bool {
        let normalized = rel_path.replace('\\', "/");
        for mp in &self.model_paths {
            let prefix = if mp.ends_with('/') {
                mp.clone()
            } else {
                format!("{mp}/")
            };
            if normalized.starts_with(&prefix) {
                return true;
            }
        }
        false
    }

    /// Extract file stem from a relative path.
    fn file_stem(rel_path: &str) -> Option<String> {
        let normalized = rel_path.replace('\\', "/");
        Path::new(&normalized)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
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
        let dbt_root = detect_dbt_project(root).ok_or_else(|| anyhow::anyhow!("no dbt project"))?;
        self.dbt_root = Some(dbt_root.clone());

        // Read model-paths from dbt_project.yml
        let project_yml = dbt_root.join("dbt_project.yml");
        if let Ok(content) = std::fs::read_to_string(&project_yml)
            && let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content)
            && let Some(serde_yaml::Value::Sequence(paths)) = yaml.get("model-paths")
        {
            self.model_paths = paths
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }

        // Parse doc blocks from model dirs and docs/ dirs
        let mut all_doc_dirs: Vec<PathBuf> = Vec::new();
        let model_paths = self.model_paths.clone();
        for mp in &model_paths {
            let model_dir = dbt_root.join(mp);
            if model_dir.exists() {
                // Walk model dir for .md files
                self.collect_doc_blocks_recursive(&model_dir);
            }
        }
        // Also check docs/ directories at dbt_root
        let docs_dir = dbt_root.join("docs");
        if docs_dir.exists() {
            all_doc_dirs.push(docs_dir);
        }
        for dd in &all_doc_dirs {
            self.doc_blocks.extend(parse_doc_blocks(dd));
        }

        // Parse YAML schema files from model dirs
        let doc_blocks_snapshot = self.doc_blocks.clone();
        for mp in &model_paths {
            let model_dir = dbt_root.join(mp);
            if model_dir.exists() {
                self.models
                    .extend(parse_yml_files(&model_dir, &doc_blocks_snapshot));
                // Also walk subdirectories for yml files
                self.collect_yml_recursive_with_blocks(&model_dir, &doc_blocks_snapshot);
            }
        }

        Ok(())
    }

    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        if !self.is_in_model_dir(rel_path) {
            return None;
        }
        let stem = Self::file_stem(rel_path)?;
        self.models.get(&stem).map(|m| m.to_file_context())
    }

    fn get_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut meta = HashMap::new();
        meta.insert(
            "dbt_doc_blocks".into(),
            serde_json::Value::Number(serde_json::Number::from(self.doc_blocks.len())),
        );
        meta.insert(
            "dbt_models".into(),
            serde_json::Value::Number(serde_json::Number::from(self.models.len())),
        );
        meta
    }
}

impl DbtContextProvider {
    fn collect_doc_blocks_recursive(&mut self, dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    self.collect_doc_blocks_recursive(&path);
                } else if path.extension().is_some_and(|e| e == "md")
                    && let Ok(content) = std::fs::read_to_string(&path)
                {
                    self.doc_blocks
                        .extend(parse_doc_blocks_from_content(&content));
                }
            }
        }
    }

    fn collect_yml_recursive_with_blocks(
        &mut self,
        dir: &Path,
        doc_blocks: &HashMap<String, String>,
    ) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    self.collect_yml_recursive_with_blocks(&path, doc_blocks);
                } else {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if (ext == "yml" || ext == "yaml")
                        && let Ok(content) = std::fs::read_to_string(&path)
                    {
                        self.models.extend(parse_yml_content(&content, doc_blocks));
                    }
                }
            }
        }
    }
}
