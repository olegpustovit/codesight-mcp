//! Domain-owned documentation indexing orchestration.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use walkdir::{DirEntry, WalkDir};

use codesight_storage::storage::vector_index::OwnedVectorRow;

use crate::{
    repo_id::normalize_repo_id,
    retrieval::sections::{extract_sections, is_doc_file},
};

use super::{DocFileUpdate, DocStore};

#[derive(Debug, Clone)]
pub struct DocIndexSummary {
    pub repo: String,
    pub indexed_at: String,
    pub file_count: usize,
    pub section_count: usize,
}

pub fn index_local_docs(db_path: PathBuf, root: &Path) -> anyhow::Result<DocIndexSummary> {
    index_local_docs_with_section_vectors(db_path, root, |_, _| Ok(Vec::new()))
}

pub fn index_local_docs_with_section_vectors<F>(
    db_path: PathBuf,
    root: &Path,
    build_section_vectors: F,
) -> anyhow::Result<DocIndexSummary>
where
    F: FnOnce(&str, &[DocFileUpdate]) -> anyhow::Result<Vec<OwnedVectorRow>>,
{
    let repo = normalize_repo_id(root);
    let store = DocStore::open(db_path)?;
    let index_version = store.current_index_version()?.saturating_add(1);
    let existing = store.file_fingerprints()?;

    let mut seen_files = HashSet::new();
    let mut updated_files = Vec::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(should_descend)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        let relative_path = match to_repo_relative(path, root) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !is_doc_file(&relative_path) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        seen_files.insert(relative_path.clone());

        let size_bytes = metadata.len();
        let modified_ns = modified_ns(&metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let content_hash = blake3::hash(&bytes).to_hex().to_string();

        if let Some(existing_file) = existing.get(&relative_path)
            && existing_file.content_hash == content_hash
        {
            continue;
        }

        let source = String::from_utf8_lossy(&bytes).into_owned();
        let sections = extract_sections(&relative_path, &source);

        let format = if relative_path.ends_with(".md") || relative_path.ends_with(".markdown") {
            "markdown"
        } else if relative_path.ends_with(".rst") {
            "rst"
        } else if relative_path.ends_with(".adoc") || relative_path.ends_with(".asciidoc") {
            "asciidoc"
        } else if relative_path.ends_with(".html") || relative_path.ends_with(".htm") {
            "html"
        } else if relative_path.ends_with(".ipynb") {
            "notebook"
        } else {
            "text"
        };

        updated_files.push(DocFileUpdate {
            file_path: relative_path,
            format: format.to_string(),
            size_bytes,
            modified_ns,
            content_hash,
            sections,
            source,
        });
    }

    let removed_files = existing
        .keys()
        .filter(|path| !seen_files.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let indexed_at = chrono::Utc::now().to_rfc3339();
    let file_count = seen_files.len();
    let section_vectors = build_section_vectors(&repo, &updated_files)?;
    store.apply_index(
        &repo,
        root,
        &indexed_at,
        index_version,
        &updated_files,
        &removed_files,
        &section_vectors,
    )?;
    // Report the total section count in the docs index after apply_index,
    // not just the sections touched by this run. The previous behavior
    // returned 0 on incremental runs where no files changed, misleading
    // callers that inspected the response body.
    let section_count = store.section_count()?;

    Ok(DocIndexSummary {
        repo,
        indexed_at,
        file_count,
        section_count,
    })
}

fn modified_ns(time: &SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

fn should_descend(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !crate::db::SKIP_DIRS.iter().any(|skip| *skip == name)
}

fn to_repo_relative(path: &Path, root: &Path) -> anyhow::Result<String> {
    let relative = path.strip_prefix(root)?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}
