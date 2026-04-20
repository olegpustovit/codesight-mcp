//! Shared row-mapping helpers for `rusqlite` queries against the symbol index.

use rusqlite::{Error, Row, types::Type};

use crate::core::symbols::{ExtractionTier, SymbolKind};

use super::{SearchSymbol, SymbolRecord};

fn bad_text(col: usize, what: &str, value: &str) -> Error {
    Error::FromSqlConversionFailure(
        col,
        Type::Text,
        Box::new(std::io::Error::other(format!("unknown {what} {value}"))),
    )
}

pub(crate) fn kind_from_row(row: &Row<'_>, col: usize) -> rusqlite::Result<SymbolKind> {
    let s: String = row.get(col)?;
    SymbolKind::from_db(&s).ok_or_else(|| bad_text(col, "symbol kind", &s))
}

pub(crate) fn tier_from_row(row: &Row<'_>, col: usize) -> rusqlite::Result<ExtractionTier> {
    let s: String = row.get(col)?;
    ExtractionTier::from_db(&s).ok_or_else(|| bad_text(col, "extraction tier", &s))
}

/// Materialize a `SearchSymbol` from a row whose 0..=6 columns are:
/// `id, kind, name, file_path, start_line, signature, summary`.
pub(crate) fn search_symbol_from_row(row: &Row<'_>, score: f64) -> rusqlite::Result<SearchSymbol> {
    Ok(SearchSymbol {
        id: row.get(0)?,
        kind: kind_from_row(row, 1)?,
        name: row.get(2)?,
        file: row.get(3)?,
        line: row.get::<_, u32>(4)?,
        signature: row.get(5)?,
        summary: row.get(6)?,
        score,
        match_type: None,
        fuzzy_similarity: None,
        edit_distance: None,
    })
}

/// Materialize a `SymbolRecord` from a row whose 0..=10 columns are:
/// `id, kind, extraction_tier, file_path, start_line, end_line, start_byte,
/// end_byte, source_hash, source_text, file_size_bytes`.
pub(crate) fn symbol_record_from_row(
    row: &Row<'_>,
    root_path: &std::path::Path,
) -> rusqlite::Result<SymbolRecord> {
    Ok(SymbolRecord {
        id: row.get(0)?,
        kind: kind_from_row(row, 1)?,
        extraction_tier: tier_from_row(row, 2)?,
        file_path: row.get(3)?,
        start_line: row.get::<_, u32>(4)?,
        end_line: row.get::<_, u32>(5)?,
        start_byte: row.get::<_, i64>(6)? as usize,
        end_byte: row.get::<_, i64>(7)? as usize,
        source_hash: row.get(8)?,
        stored_source: row.get(9)?,
        file_size_bytes: row.get::<_, i64>(10)? as u64,
        root_path: root_path.to_path_buf(),
    })
}

/// Normalize a user-provided scope (strip stray `*`, normalize `\` to `/`).
pub(crate) fn normalize_scope(scope: &str) -> String {
    scope
        .replace('\\', "/")
        .trim_matches('*')
        .trim()
        .to_string()
}

/// Append an `AND (col = ? OR col LIKE ? OR col LIKE ?)` clause matching an
/// optional scope (exact / prefix / contains). No-op when `scope` is `None`.
pub(crate) fn push_scope_clause(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    column: &str,
    scope: Option<&str>,
) {
    let Some(scope) = scope else {
        return;
    };
    let scope = normalize_scope(scope);
    sql.push_str(&format!(
        " AND ({column} = ? OR {column} LIKE ? OR {column} LIKE ?)"
    ));
    params.push(rusqlite::types::Value::from(scope.clone()));
    params.push(rusqlite::types::Value::from(format!("{scope}%")));
    params.push(rusqlite::types::Value::from(format!("%{scope}%")));
}

/// Append `AND (col NOT LIKE ? AND col NOT LIKE ?)` for each exclude pattern.
pub(crate) fn push_exclude_clause(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    column: &str,
    exclude: &[String],
) {
    for pattern in exclude {
        let pattern = normalize_scope(pattern);
        sql.push_str(&format!(
            " AND ({column} NOT LIKE ? AND {column} NOT LIKE ?)"
        ));
        params.push(rusqlite::types::Value::from(format!("{pattern}%")));
        params.push(rusqlite::types::Value::from(format!("%{pattern}%")));
    }
}
