//! Reusable indexing helpers for integration and slice tests.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::AppConfig;
use crate::indexing::code_index::{CodeIndexOutcome, index_workspace};
use crate::repo_id::normalize_repo_id;
use crate::service::CodeService;
use crate::storage::{IndexStore, doc_index, doc_index::DocStore};

/// Write a set of repo-relative files under `root`.
pub fn write_files(root: &Path, files: &[(&str, &str)]) -> Result<()> {
    for (rel, content) in files {
        let path = root.join(rel);
        let parent = path
            .parent()
            .with_context(|| format!("file path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)?;
        fs::write(&path, content)?;
    }
    Ok(())
}

/// Recursively copy a checked-in fixture tree into `dst`.
pub fn copy_fixture_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).with_context(|| {
            format!(
                "strip prefix {} from {}",
                src.display(),
                entry.path().display()
            )
        })?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        let parent = target
            .parent()
            .with_context(|| format!("fixture target has no parent: {}", target.display()))?;
        fs::create_dir_all(parent)?;
        fs::copy(entry.path(), &target)?;
    }
    Ok(())
}

/// Create a fresh service rooted at `index_root`.
pub fn make_service(index_root: &Path) -> Result<CodeService> {
    CodeService::new(AppConfig::new(index_root.to_path_buf()))
}

/// Index a source repo directly through the domain orchestrator and return its source id.
pub fn index_code_fixture(
    index_root: &Path,
    repo_root: &Path,
) -> Result<(AppConfig, String, CodeIndexOutcome)> {
    let config = AppConfig::new(index_root.to_path_buf());
    let outcome = index_workspace(&config, repo_root)?;
    let source_id = outcome.summary.repo.clone();
    Ok((config, source_id, outcome))
}

/// Open an already-indexed code store for `source_id`.
pub fn open_code_store(config: &AppConfig, source_id: &str) -> Result<IndexStore> {
    IndexStore::open_existing(config.repo_db_path(source_id))?
        .with_context(|| format!("indexed code store missing for {source_id}"))
}

/// Index a documentation repo directly through the doc-index orchestrator and return its source id.
pub fn index_doc_fixture(
    index_root: &Path,
    repo_root: &Path,
) -> Result<(AppConfig, String, doc_index::DocIndexSummary)> {
    let config = AppConfig::new(index_root.to_path_buf());
    let summary =
        doc_index::index_local_docs(config.doc_db_path(&normalize_repo_id(repo_root)), repo_root)?;
    let source_id = summary.repo.clone();
    Ok((config, source_id, summary))
}

/// Open an already-indexed doc store for `source_id`.
pub fn open_doc_store(config: &AppConfig, source_id: &str) -> Result<DocStore> {
    DocStore::open_existing(config.doc_db_path(source_id))?
        .with_context(|| format!("indexed doc store missing for {source_id}"))
}

/// Find a symbol id by exact symbol name.
pub fn find_symbol_id(store: &IndexStore, name: &str) -> Result<String> {
    let results = store.search_symbols_name_contains(name, None, None, 20)?;
    results
        .into_iter()
        .find(|symbol| symbol.name == name)
        .map(|symbol| symbol.id)
        .ok_or_else(|| anyhow::anyhow!("symbol {name:?} not found"))
}

/// Resolve a repo-local relative path into a canonical slash-normalized string.
pub fn normalize_relative_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} is not under {}", path.display(), root.display()))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        bail!("normalized path is empty for {}", path.display());
    }
    Ok(rel)
}

/// Convenience for `index_root/repo`.
pub fn repo_root(workspace: &Path, name: &str) -> PathBuf {
    workspace.join(name)
}
