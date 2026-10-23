//! DAO for the `reachability_edges` table.
//!
//! Persists per-file structural edges produced by `Language::reachability_edges`
//! at ingest time. The dead-code analysis crate reads from this table instead
//! of re-scanning every file's text on each call.

use rusqlite::{Connection, params};

use dowser_core::reachability::{EdgeKind, StructuralEdge};

use crate::error::StorageResult;

/// Replace every edge whose `source_file = file` with the given set in a
/// single transaction. Caller is expected to wrap multiple files in a
/// `tx.commit()` boundary.
pub fn replace_edges_for_file(
    conn: &Connection,
    source_file: &str,
    edges: &[StructuralEdge],
) -> StorageResult<()> {
    {
        let mut del =
            conn.prepare_cached("DELETE FROM reachability_edges WHERE source_file = ?1")?;
        del.execute([source_file])?;
    }
    if edges.is_empty() {
        return Ok(());
    }
    let mut ins = conn.prepare_cached(
        "INSERT OR IGNORE INTO reachability_edges \
            (source_file, target_file, target_symbol, edge_kind) \
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for edge in edges {
        if !edge.is_useful() {
            continue;
        }
        ins.execute(params![
            source_file,
            edge.target_file.as_deref(),
            edge.target_symbol.as_deref(),
            edge.edge_kind.as_str(),
        ])?;
    }
    Ok(())
}

/// Delete every edge whose source or target file is `file`. Used by the
/// removed-files cleanup path during `apply_index_with_deps`.
pub fn delete_edges_for_file(conn: &Connection, file: &str) -> StorageResult<()> {
    let mut del = conn.prepare_cached(
        "DELETE FROM reachability_edges WHERE source_file = ?1 OR target_file = ?1",
    )?;
    del.execute([file])?;
    Ok(())
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StructuralEdge> {
    let source_file: String = row.get(0)?;
    let target_file: Option<String> = row.get(1)?;
    let target_symbol: Option<String> = row.get(2)?;
    let edge_kind: String = row.get(3)?;
    let edge_kind = EdgeKind::parse_kind(&edge_kind).unwrap_or(EdgeKind::ImportStatement);
    Ok(StructuralEdge {
        source_file,
        target_file,
        target_symbol,
        edge_kind,
    })
}

pub fn edges_targeting_file(
    conn: &Connection,
    target_file: &str,
) -> StorageResult<Vec<StructuralEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_file, target_file, target_symbol, edge_kind \
         FROM reachability_edges WHERE target_file = ?1",
    )?;
    let rows = stmt.query_map([target_file], map_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn edges_targeting_symbol(
    conn: &Connection,
    target_symbol: &str,
) -> StorageResult<Vec<StructuralEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_file, target_file, target_symbol, edge_kind \
         FROM reachability_edges WHERE target_symbol = ?1",
    )?;
    let rows = stmt.query_map([target_symbol], map_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn edges_from_file(conn: &Connection, source_file: &str) -> StorageResult<Vec<StructuralEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_file, target_file, target_symbol, edge_kind \
         FROM reachability_edges WHERE source_file = ?1",
    )?;
    let rows = stmt.query_map([source_file], map_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Return every edge whose `edge_kind` is in the given set. When the set
/// is empty, returns the entire table.
pub fn all_edges_for_kinds(
    conn: &Connection,
    kinds: &[EdgeKind],
) -> StorageResult<Vec<StructuralEdge>> {
    if kinds.is_empty() {
        let mut stmt = conn.prepare_cached(
            "SELECT source_file, target_file, target_symbol, edge_kind \
             FROM reachability_edges",
        )?;
        let rows = stmt.query_map([], map_row)?;
        return Ok(rows.collect::<Result<_, _>>()?);
    }
    let placeholders = std::iter::repeat_n("?", kinds.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT source_file, target_file, target_symbol, edge_kind \
         FROM reachability_edges WHERE edge_kind IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let kind_strs: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    let params = rusqlite::params_from_iter(kind_strs.iter());
    let rows = stmt.query_map(params, map_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn open_in_memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::REPO_INFO_DDL).unwrap();
        conn.execute_batch(
            "CREATE TABLE files (file_path TEXT PRIMARY KEY, language TEXT NOT NULL, \
                size_bytes INTEGER NOT NULL, modified_ns INTEGER NOT NULL, content_hash TEXT NOT NULL);",
        )
        .unwrap();
        super::super::migrations::apply_code_index_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn replace_then_query() {
        let conn = open_in_memory();
        // files table needs entries because of FK.
        conn.execute("INSERT INTO files VALUES ('a.rs','rust',0,0,'x')", [])
            .unwrap();
        conn.execute("INSERT INTO files VALUES ('b.rs','rust',0,0,'x')", [])
            .unwrap();

        let edges = vec![
            StructuralEdge::new_file("a.rs", "b.rs", EdgeKind::ImportStatement),
            StructuralEdge::new_symbol("a.rs", "do_thing", EdgeKind::MatchDispatch),
        ];
        replace_edges_for_file(&conn, "a.rs", &edges).unwrap();

        let by_target = edges_targeting_file(&conn, "b.rs").unwrap();
        assert_eq!(by_target.len(), 1);
        assert_eq!(by_target[0].source_file, "a.rs");

        let by_symbol = edges_targeting_symbol(&conn, "do_thing").unwrap();
        assert_eq!(by_symbol.len(), 1);
        assert_eq!(by_symbol[0].edge_kind, EdgeKind::MatchDispatch);

        let from_a = edges_from_file(&conn, "a.rs").unwrap();
        assert_eq!(from_a.len(), 2);
    }

    #[test]
    fn replace_overwrites_previous_set() {
        let conn = open_in_memory();
        conn.execute("INSERT INTO files VALUES ('a.rs','rust',0,0,'x')", [])
            .unwrap();
        replace_edges_for_file(
            &conn,
            "a.rs",
            &[StructuralEdge::new_symbol(
                "a.rs",
                "old",
                EdgeKind::ReExport,
            )],
        )
        .unwrap();
        replace_edges_for_file(
            &conn,
            "a.rs",
            &[StructuralEdge::new_symbol(
                "a.rs",
                "new",
                EdgeKind::ReExport,
            )],
        )
        .unwrap();
        assert!(edges_targeting_symbol(&conn, "old").unwrap().is_empty());
        assert_eq!(edges_targeting_symbol(&conn, "new").unwrap().len(), 1);
    }
}
