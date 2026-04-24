//! BM25 and lightweight symbol search with token overlap reranking.

use std::collections::HashSet;

use rusqlite::{named_params, types::Value as SqlValue};

use crate::db;
use crate::storage::SearchSymbol;

use super::IndexStore;
use super::row_utils::{
    normalize_scope, push_exclude_clause, push_scope_clause, search_symbol_from_row,
};
use super::types::SearchCandidate;

impl IndexStore {
    pub fn search_symbols(
        &self,
        query: &str,
        kind: Option<&str>,
        scope: Option<&str>,
        exclude: &[String],
        max_results: usize,
    ) -> anyhow::Result<Vec<SearchSymbol>> {
        let query_tokens = db::tokenize(query);
        if query_tokens.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = query_tokens
            .iter()
            .map(|token| format!("{token}*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let row_limit = (max_results.clamp(1, 25) * 6) as i64;

        let conn = self.open_read_only()?;
        let mut sql = String::from(
            r#"
            SELECT
                s.id,
                s.kind,
                s.name,
                s.file_path,
                s.start_line,
                s.signature,
                s.summary,
                s.qualified_name,
                bm25(symbol_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0) AS rank
            FROM symbol_fts
            JOIN symbols s ON s.row_id = symbol_fts.rowid
            WHERE symbol_fts MATCH ?
        "#,
        );
        let mut params = vec![SqlValue::from(fts_query)];
        if let Some(kind) = kind {
            sql.push_str(" AND s.kind = ?");
            params.push(SqlValue::from(kind.to_string()));
        }
        push_scope_clause(&mut sql, &mut params, "s.file_path", scope);
        push_exclude_clause(&mut sql, &mut params, "s.file_path", exclude);
        sql.push_str(" ORDER BY rank ASC LIMIT ?");
        params.push(SqlValue::Integer(row_limit));
        let mut stmt = conn.prepare_cached(&sql)?;

        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(SearchCandidate {
                result: search_symbol_from_row(row, 0.0)?,
                qualified_name: row.get(7)?,
                raw_rank: row.get::<_, f64>(8)?,
            })
        })?;

        let mut seen_ids = HashSet::new();
        let mut candidates = Vec::new();
        for row in rows {
            let candidate = row?;
            if seen_ids.insert(candidate.result.id.clone()) {
                candidates.push(candidate);
            }
        }

        let mut reranked = candidates
            .into_iter()
            .map(|candidate| rerank_candidate(candidate, &query_tokens))
            .collect::<Vec<_>>();
        reranked.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.id.cmp(&right.id))
        });
        reranked.truncate(max_results.clamp(1, 25));
        Ok(reranked)
    }

    /// True pre-cap cardinality for the same BM25 WHERE clause as `search_symbols`.
    /// Returns `(total_matches, files_with_matches)`.
    pub fn count_symbols(
        &self,
        query: &str,
        kind: Option<&str>,
        scope: Option<&str>,
        exclude: &[String],
    ) -> anyhow::Result<(usize, usize)> {
        let query_tokens = db::tokenize(query);
        if query_tokens.is_empty() {
            return Ok((0, 0));
        }

        let fts_query = query_tokens
            .iter()
            .map(|token| format!("{token}*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let conn = self.open_read_only()?;
        let mut sql = String::from(
            r#"
            SELECT COUNT(DISTINCT s.id), COUNT(DISTINCT s.file_path)
            FROM symbol_fts
            JOIN symbols s ON s.row_id = symbol_fts.rowid
            WHERE symbol_fts MATCH ?
        "#,
        );
        let mut params = vec![SqlValue::from(fts_query)];
        if let Some(kind) = kind {
            sql.push_str(" AND s.kind = ?");
            params.push(SqlValue::from(kind.to_string()));
        }
        push_scope_clause(&mut sql, &mut params, "s.file_path", scope);
        push_exclude_clause(&mut sql, &mut params, "s.file_path", exclude);
        let mut stmt = conn.prepare_cached(&sql)?;

        let (total, files): (i64, i64) = stmt
            .query_row(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        Ok((total.max(0) as usize, files.max(0) as usize))
    }

    pub fn sum_file_sizes_for_paths(&self, paths: &[String]) -> anyhow::Result<u64> {
        if paths.is_empty() {
            return Ok(0);
        }

        let placeholders = std::iter::repeat_n("?", paths.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM files WHERE file_path IN ({placeholders})"
        );
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(&sql)?;
        let total = stmt.query_row(rusqlite::params_from_iter(paths.iter()), |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(total.max(0) as u64)
    }

    /// Substring name match (case-insensitive) for `search_catalog` fuzzy augmentation.
    pub fn search_symbols_name_contains(
        &self,
        needle: &str,
        kind: Option<&str>,
        scope: Option<&str>,
        max_results: usize,
    ) -> anyhow::Result<Vec<SearchSymbol>> {
        let trimmed = needle.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        self.scan_symbols_by_scope(Some(trimmed), kind, scope, max_results.clamp(1, 100), 0.42)
    }

    /// Materialize search-shaped symbol rows for a specific id set.
    pub fn search_symbols_by_ids(&self, ids: &[String]) -> anyhow::Result<Vec<SearchSymbol>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, kind, name, file_path, start_line, signature, summary
             FROM symbols
             WHERE id IN ({placeholders})"
        );
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            search_symbol_from_row(row, 0.0)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Bounded scan of symbols for fuzzy / relatedness tooling (not BM25-ranked).
    pub fn symbols_light_scan(
        &self,
        kind: Option<&str>,
        scope: Option<&str>,
        max_rows: usize,
    ) -> anyhow::Result<Vec<SearchSymbol>> {
        self.scan_symbols_by_scope(None, kind, scope, max_rows.clamp(1, 20_000), 0.0)
    }

    /// Shared body for `search_symbols_name_contains` and `symbols_light_scan`.
    /// `needle = Some(s)` filters by case-insensitive name substring; `None`
    /// scans every row in the scope.
    fn scan_symbols_by_scope(
        &self,
        needle: Option<&str>,
        kind: Option<&str>,
        scope: Option<&str>,
        limit: usize,
        score: f64,
    ) -> anyhow::Result<Vec<SearchSymbol>> {
        let scope_value = scope.map(normalize_scope);
        let scope_prefix = scope_value.as_ref().map(|value| format!("{value}%"));
        let scope_contains = scope_value.as_ref().map(|value| format!("%{value}%"));
        let lim = limit as i64;

        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT
                s.id,
                s.kind,
                s.name,
                s.file_path,
                s.start_line,
                s.signature,
                s.summary
            FROM symbols s
            WHERE (:needle IS NULL OR instr(lower(s.name), lower(:needle)) > 0)
              AND (:kind IS NULL OR s.kind = :kind)
              AND (
                    :scope_exact IS NULL
                 OR s.file_path = :scope_exact
                 OR s.file_path LIKE :scope_prefix
                 OR s.file_path LIKE :scope_contains
              )
            ORDER BY s.file_path, s.start_line, s.id
            LIMIT :lim
        "#,
        )?;

        let rows = stmt.query_map(
            named_params! {
                ":needle": &needle,
                ":kind": &kind,
                ":scope_exact": &scope_value.as_deref(),
                ":scope_prefix": &scope_prefix.as_deref(),
                ":scope_contains": &scope_contains.as_deref(),
                ":lim": &lim,
            },
            |row| search_symbol_from_row(row, score),
        )?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn rerank_candidate(candidate: SearchCandidate, query_tokens: &[String]) -> SearchSymbol {
    let name_tokens = db::tokenize(&candidate.result.name);
    let qualified_tokens = db::tokenize(&candidate.qualified_name);
    let path_tokens = db::tokenize(&candidate.result.file);
    let summary_tokens = db::tokenize(&format!(
        "{} {} {}",
        candidate.result.summary, candidate.result.signature, candidate.qualified_name
    ));

    let name_overlap = overlap(query_tokens, &name_tokens);
    let qualified_overlap = overlap(query_tokens, &qualified_tokens);
    let path_overlap = overlap(query_tokens, &path_tokens);
    let summary_overlap = overlap(query_tokens, &summary_tokens);
    let fts_score = 1.0 / (1.0 + candidate.raw_rank.max(0.0));
    let exact_phrase = if format!(
        "{} {} {} {}",
        candidate.result.file,
        candidate.qualified_name,
        candidate.result.signature,
        candidate.result.summary
    )
    .to_ascii_lowercase()
    .contains(&query_tokens.join(" "))
    {
        0.08
    } else {
        0.0
    };

    let mut result = candidate.result;
    result.score = (0.35 * name_overlap
        + 0.20 * qualified_overlap
        + 0.20 * path_overlap
        + 0.15 * summary_overlap
        + 0.10 * fts_score
        + exact_phrase)
        .min(1.0);
    result.match_type = None;
    result.fuzzy_similarity = None;
    result.edit_distance = None;
    result
}

fn overlap(query_tokens: &[String], candidate_tokens: &[String]) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let candidate_set = candidate_tokens.iter().collect::<HashSet<_>>();
    let matched = query_tokens
        .iter()
        .filter(|token| candidate_set.contains(token))
        .count();
    matched as f64 / query_tokens.len() as f64
}
