//! Dense-embedding helpers backed by `sqlite-vec` virtual tables.
//!
//! Storage owns the row_id ↔ stable_id resolution and transactional writes.
//! The root crate owns the embedder and chunk-text policy.

use rusqlite::{Connection, params, types::ValueRef};

use crate::error::StorageResult;

use super::schema::EMBEDDING_DIM;

/// One (stable_id, embedding) pair for batched inserts. The same shape is
/// reused for both code symbols and doc sections.
pub struct VectorRow<'a> {
    pub stable_id: &'a str,
    pub embedding: &'a [f32],
}

pub struct OwnedVectorRow {
    pub stable_id: String,
    pub embedding: Vec<f32>,
}

impl OwnedVectorRow {
    pub fn borrowed(&self) -> VectorRow<'_> {
        VectorRow {
            stable_id: &self.stable_id,
            embedding: &self.embedding,
        }
    }
}

/// Which domain a vec operation targets. Drives table names + the FK
/// column that joins back to the lexical row.
#[derive(Clone, Copy)]
pub enum Domain {
    Symbol,
    Section,
}

impl Domain {
    fn tables(self) -> (&'static str, &'static str, &'static str) {
        match self {
            // (lexical_table, vec_table, vec_pk_col)
            Self::Symbol => ("symbols", "vec_symbol_embeddings", "symbol_rowid"),
            Self::Section => ("sections", "vec_section_embeddings", "section_rowid"),
        }
    }
}

fn vec_blob(embedding: &[f32]) -> StorageResult<Vec<u8>> {
    if embedding.len() != EMBEDDING_DIM {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "embedding dim {} != expected {EMBEDDING_DIM}",
            embedding.len()
        ))
        .into());
    }
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(out)
}

/// Replace vec rows for the given stable IDs. Row IDs are resolved from
/// the lexical table inside the same connection. Missing stable IDs are
/// silently skipped — the lexical insert is the source of truth.
pub fn upsert_vectors(
    conn: &Connection,
    domain: Domain,
    rows: &[VectorRow<'_>],
) -> StorageResult<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let (lex, vec, pk) = domain.tables();
    let mut id_stmt = conn.prepare_cached(&format!("SELECT row_id FROM {lex} WHERE id = ?1"))?;
    let mut del_stmt = conn.prepare_cached(&format!("DELETE FROM {vec} WHERE {pk} = ?1"))?;
    let mut ins_stmt = conn.prepare_cached(&format!(
        "INSERT INTO {vec}({pk}, embedding) VALUES (?1, ?2)"
    ))?;
    let mut written = 0usize;
    for row in rows {
        let Ok(row_id) = id_stmt.query_row(params![row.stable_id], |r| r.get::<_, i64>(0)) else {
            continue;
        };
        del_stmt.execute(params![row_id])?;
        ins_stmt.execute(params![row_id, vec_blob(row.embedding)?])?;
        written += 1;
    }
    Ok(written)
}

/// Delete vec rows by stable id. Unknown IDs are silently ignored.
pub fn delete_vectors(
    conn: &Connection,
    domain: Domain,
    stable_ids: &[&str],
) -> StorageResult<usize> {
    if stable_ids.is_empty() {
        return Ok(0);
    }
    let (lex, vec, pk) = domain.tables();
    let mut id_stmt = conn.prepare_cached(&format!("SELECT row_id FROM {lex} WHERE id = ?1"))?;
    let mut del_stmt = conn.prepare_cached(&format!("DELETE FROM {vec} WHERE {pk} = ?1"))?;
    let mut n = 0usize;
    for sid in stable_ids {
        if let Ok(row_id) = id_stmt.query_row(params![*sid], |r| r.get::<_, i64>(0)) {
            n += del_stmt.execute(params![row_id])? as usize;
        }
    }
    Ok(n)
}

/// KNN over dense embeddings. Joins back to the lexical table so callers
/// get stable IDs, not raw row IDs.
pub fn query_knn(
    conn: &Connection,
    domain: Domain,
    query: &[f32],
    k: usize,
) -> StorageResult<Vec<(String, f64)>> {
    let (lex, vec, pk) = domain.tables();
    let sql = format!(
        "SELECT s.id, v.distance \
         FROM {vec} v JOIN {lex} s ON s.row_id = v.{pk} \
         WHERE v.embedding MATCH ?1 AND k = ?2 \
         ORDER BY v.distance"
    );
    let blob = vec_blob(query)?;
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params![blob, k as i64])?;
    let mut out = Vec::with_capacity(k);
    while let Some(row) = rows.next()? {
        let id: String = match row.get_ref(0)? {
            ValueRef::Text(t) => std::str::from_utf8(t).unwrap_or_default().to_string(),
            _ => continue,
        };
        let dist: f64 = row.get(1)?;
        out.push((id, dist));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn sqlite_vec_available_and_roundtrips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.sqlite3");
        let conn = db::open_rw(&path).unwrap();

        let vec_ver: String = conn
            .query_row("SELECT vec_version()", [], |r| r.get(0))
            .unwrap();
        assert!(!vec_ver.is_empty(), "vec_version should be non-empty");

        conn.execute_batch(
            "CREATE TABLE symbols (row_id INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE);
             CREATE VIRTUAL TABLE vec_symbol_embeddings USING vec0(symbol_rowid INTEGER PRIMARY KEY, embedding FLOAT[256]);",
        )
        .unwrap();
        conn.execute("INSERT INTO symbols(id) VALUES('a'), ('b')", [])
            .unwrap();

        let mut a = vec![0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = vec![0.0f32; EMBEDDING_DIM];
        b[1] = 1.0;
        upsert_vectors(
            &conn,
            Domain::Symbol,
            &[
                VectorRow {
                    stable_id: "a",
                    embedding: &a,
                },
                VectorRow {
                    stable_id: "b",
                    embedding: &b,
                },
            ],
        )
        .unwrap();

        let hits = query_knn(&conn, Domain::Symbol, &a, 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "a", "nearest neighbor of a must be a");
    }
}
