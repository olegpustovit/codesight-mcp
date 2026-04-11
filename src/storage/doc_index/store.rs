//! Write-side operations for the doc index: delete-then-insert transactions.

use std::path::Path;

use rusqlite::{Transaction, params};

use crate::{config::SCHEMA_VERSION, retrieval::sections::Section};

use super::{DocFileUpdate, DocStore};

impl DocStore {
    pub fn apply_index(
        &self,
        repo: &str,
        root_path: &Path,
        indexed_at: &str,
        index_version: u32,
        updated_files: &[DocFileUpdate],
        removed_files: &[String],
    ) -> anyhow::Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        for file_path in removed_files {
            tx.execute("DELETE FROM sections WHERE file_path = ?1", [file_path])?;
            tx.execute("DELETE FROM doc_files WHERE file_path = ?1", [file_path])?;
        }

        for file in updated_files {
            tx.execute(
                "DELETE FROM sections WHERE file_path = ?1",
                [&file.file_path],
            )?;
            tx.execute(
                "DELETE FROM doc_files WHERE file_path = ?1",
                [&file.file_path],
            )?;
            tx.execute(
                "INSERT INTO doc_files (file_path, format, size_bytes, modified_ns, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![file.file_path, file.format, file.size_bytes as i64, file.modified_ns as i64, file.content_hash],
            )?;
            for section in &file.sections {
                Self::insert_section(&tx, section, &file.source)?;
            }
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
        tx.execute(
            r#"INSERT INTO sections (
                id, file_path, heading, heading_path, level, summary, content_text,
                start_line, end_line, start_byte, end_byte, content_hash, file_size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
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
            ],
        )?;
        Ok(())
    }
}
