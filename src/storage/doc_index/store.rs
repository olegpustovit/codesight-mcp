//! Write-side operations for the doc index: delete-then-insert transactions.

use std::path::Path;

use rusqlite::{Transaction, params};

use codesight_retrieval::sections::Section;

use crate::config::SCHEMA_VERSION;

use super::{DocFileUpdate, DocStore};

impl DocStore {
    #[allow(clippy::too_many_arguments)]
    pub fn apply_index(
        &self,
        repo: &str,
        root_path: &Path,
        indexed_at: &str,
        index_version: u32,
        updated_files: &[DocFileUpdate],
        removed_files: &[String],
        section_vectors: &[codesight_storage::storage::vector_index::OwnedVectorRow],
    ) -> anyhow::Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        if !removed_files.is_empty() {
            let mut del_sec = tx.prepare_cached("DELETE FROM sections WHERE file_path = ?1")?;
            let mut del_doc = tx.prepare_cached("DELETE FROM doc_files WHERE file_path = ?1")?;
            for file_path in removed_files {
                Self::delete_section_vectors_for_file(&tx, file_path)?;
                del_sec.execute([file_path])?;
                del_doc.execute([file_path])?;
            }
        }

        for file in updated_files {
            Self::delete_section_vectors_for_file(&tx, &file.file_path)?;
            tx.prepare_cached("DELETE FROM sections WHERE file_path = ?1")?
                .execute([&file.file_path])?;
            tx.prepare_cached("DELETE FROM doc_files WHERE file_path = ?1")?
                .execute([&file.file_path])?;
            tx.prepare_cached(
                "INSERT INTO doc_files (file_path, format, size_bytes, modified_ns, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![file.file_path, file.format, file.size_bytes as i64, file.modified_ns as i64, file.content_hash])?;
            for section in &file.sections {
                Self::insert_section(&tx, section, &file.source)?;
            }
        }
        if !section_vectors.is_empty() {
            let rows: Vec<_> = section_vectors
                .iter()
                .map(codesight_storage::storage::vector_index::OwnedVectorRow::borrowed)
                .collect();
            codesight_storage::storage::vector_index::upsert_vectors(
                &tx,
                codesight_storage::storage::vector_index::Domain::Section,
                &rows,
            )?;
        }

        tx.execute("DELETE FROM repo_info", [])?;
        tx.execute(
            "INSERT INTO repo_info (repo, root_path, indexed_at, index_version, schema_version) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![repo, root_path.to_string_lossy().to_string(), indexed_at, index_version, SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn insert_section(tx: &Transaction<'_>, section: &Section, source: &str) -> anyhow::Result<()> {
        let content_text = source
            .get(section.start_byte..section.end_byte)
            .unwrap_or("");
        tx.prepare_cached(
            r#"INSERT INTO sections (
                id, file_path, heading, heading_path, level, summary, content_text,
                start_line, end_line, start_byte, end_byte, content_hash, file_size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
        )?
        .execute(params![
            section.id,
            section.file_path,
            section.heading,
            section.heading_path,
            section.level as i64,
            section.summary,
            content_text,
            section.start_line,
            section.end_line,
            section.start_byte as i64,
            section.end_byte as i64,
            section.content_hash,
            source.len() as i64,
        ])?;
        Ok(())
    }

    fn delete_section_vectors_for_file(
        tx: &Transaction<'_>,
        file_path: &str,
    ) -> anyhow::Result<()> {
        tx.execute(
            "DELETE FROM vec_section_embeddings
             WHERE section_rowid IN (SELECT row_id FROM sections WHERE file_path = ?1)",
            [file_path],
        )?;
        Ok(())
    }
}
