//! Documentation indexing orchestration: crawl, parse sections, persist.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ignore::WalkBuilder;
use rustc_hash::FxHashSet;

use codesight_language_support::is_default_excluded;
use codesight_retrieval::sections::{Section, extract_sections, is_doc_file};
use codesight_storage::db::SKIP_DIRS;
use codesight_storage::storage::doc_index::{
    DocFileUpdate, DocIndexSummary, DocSectionInput, DocStore,
};
use codesight_storage::storage::vector_index::OwnedVectorRow;

use crate::repo_id::normalize_repo_id;
use crate::service::helpers::to_repo_relative;

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

    let mut seen_files = FxHashSet::default();
    let mut updated_files = Vec::new();

    // Use ripgrep's `ignore` walker: honors `.gitignore` / `.ignore`,
    // skips hidden by default, and supports custom directory blocks via
    // `add_custom_ignore_filename`. Belt-and-suspenders: still consult the
    // workspace's `is_default_excluded` per file so `assets/`, oversized
    // tokenizer JSON, etc. never reach the doc index even when an unusual
    // gitignore lets them through.
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .filter_entry(|entry| {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| *skip == name)
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
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

        let size_bytes = metadata.len();
        // Skip vendored asset trees and oversized JSON dumps (e.g.
        // `assets/embedder/*/tokenizer.json` — 95k bogus h-sections per
        // file when surfaced as a doc). The detector unions every
        // language's vendor segments plus universal conventions
        // (`.git`, `vendor`, `assets`, `fixtures`, …).
        if is_default_excluded(&relative_path, Some(size_bytes)) {
            continue;
        }
        seen_files.insert(relative_path.clone());
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
        let sections = extract_sections(&relative_path, &source)
            .into_iter()
            .map(section_to_input)
            .collect();

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
    store.apply_doc_index(
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

fn section_to_input(section: Section) -> DocSectionInput {
    DocSectionInput {
        id: section.id,
        file_path: section.file_path,
        heading: section.heading,
        heading_path: section.heading_path,
        level: section.level,
        start_line: section.start_line,
        end_line: section.end_line,
        start_byte: section.start_byte,
        end_byte: section.end_byte,
        content_hash: section.content_hash,
        summary: section.summary,
    }
}

fn modified_ns(time: &SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}
