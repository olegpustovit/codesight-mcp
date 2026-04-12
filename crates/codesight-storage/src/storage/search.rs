//! BM25 and lightweight symbol search with token overlap reranking.

use std::collections::HashSet;

use rusqlite::named_params;

use crate::core::symbols::SymbolKind;
use crate::db;
use crate::storage::SearchSymbol;

use super::IndexStore;
use super::types::SearchCandidate;

impl IndexStore {
    pub fn search_symbols(
        &self,
        query: &str,
        kind: Option<&str>,
        scope: Option<&str>,
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
        let scope_value = scope.map(normalize_scope);
        let scope_prefix = scope_value.as_ref().map(|value| format!("{value}%"));
        let scope_contains = scope_value.as_ref().map(|value| format!("%{value}%"));
        let row_limit = (max_results.clamp(1, 25) * 6) as i64;

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
                s.summary,
                s.qualified_name,
                bm25(symbol_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0) AS rank
            FROM symbol_fts
            JOIN symbols s ON s.row_id = symbol_fts.rowid
            WHERE symbol_fts MATCH :fts
              AND (:kind IS NULL OR s.kind = :kind)
              AND (
                    :scope_exact IS NULL
                 OR s.file_path = :scope_exact
                 OR s.file_path LIKE :scope_prefix
                 OR s.file_path LIKE :scope_contains
              )
            ORDER BY rank ASC
            LIMIT :row_limit
        "#,
        )?;

        let rows = stmt.query_map(
            named_params! {
                ":fts": &fts_query,
                ":kind": &kind,
                ":scope_exact": &scope_value.as_deref(),
                ":scope_prefix": &scope_prefix.as_deref(),
                ":scope_contains": &scope_contains.as_deref(),
                ":row_limit": &row_limit,
            },
            |row| {
                let kind_str: String = row.get(1)?;
                let kind = SymbolKind::from_db(&kind_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown symbol kind {kind_str}"
                        ))),
                    )
                })?;
                Ok(SearchCandidate {
                    result: SearchSymbol {
                        id: row.get(0)?,
                        kind,
                        name: row.get(2)?,
                        file: row.get(3)?,
                        line: row.get::<_, u32>(4)?,
                        signature: row.get(5)?,
                        summary: row.get(6)?,
                        score: 0.0,
                        match_type: None,
                        fuzzy_similarity: None,
                        edit_distance: None,
                    },
                    qualified_name: row.get(7)?,
                    raw_rank: row.get::<_, f64>(8)?,
                })
            },
        )?;

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
        let scope_value = scope.map(normalize_scope);
        let scope_prefix = scope_value.as_ref().map(|value| format!("{value}%"));
        let scope_contains = scope_value.as_ref().map(|value| format!("%{value}%"));
        let lim = max_results.clamp(1, 100) as i64;

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
            WHERE instr(lower(s.name), lower(:needle)) > 0
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
                ":needle": trimmed,
                ":kind": &kind,
                ":scope_exact": &scope_value.as_deref(),
                ":scope_prefix": &scope_prefix.as_deref(),
                ":scope_contains": &scope_contains.as_deref(),
                ":lim": &lim,
            },
            |row| {
                let kind_str: String = row.get(1)?;
                let kind = SymbolKind::from_db(&kind_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown symbol kind {kind_str}"
                        ))),
                    )
                })?;
                Ok(SearchSymbol {
                    id: row.get(0)?,
                    kind,
                    name: row.get(2)?,
                    file: row.get(3)?,
                    line: row.get::<_, u32>(4)?,
                    signature: row.get(5)?,
                    summary: row.get(6)?,
                    score: 0.42,
                    match_type: None,
                    fuzzy_similarity: None,
                    edit_distance: None,
                })
            },
        )?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Bounded scan of symbols for fuzzy / relatedness tooling (not BM25-ranked).
    pub fn symbols_light_scan(
        &self,
        kind: Option<&str>,
        scope: Option<&str>,
        max_rows: usize,
    ) -> anyhow::Result<Vec<SearchSymbol>> {
        let lim = max_rows.clamp(1, 20_000) as i64;
        let scope_value = scope.map(normalize_scope);
        let scope_prefix = scope_value.as_ref().map(|value| format!("{value}%"));
        let scope_contains = scope_value.as_ref().map(|value| format!("%{value}%"));

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
            WHERE (:kind IS NULL OR s.kind = :kind)
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
                ":kind": &kind,
                ":scope_exact": &scope_value.as_deref(),
                ":scope_prefix": &scope_prefix.as_deref(),
                ":scope_contains": &scope_contains.as_deref(),
                ":lim": &lim,
            },
            |row| {
                let kind_str: String = row.get(1)?;
                let kind = SymbolKind::from_db(&kind_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown symbol kind {kind_str}"
                        ))),
                    )
                })?;
                Ok(SearchSymbol {
                    id: row.get(0)?,
                    kind,
                    name: row.get(2)?,
                    file: row.get(3)?,
                    line: row.get::<_, u32>(4)?,
                    signature: row.get(5)?,
                    summary: row.get(6)?,
                    score: 0.0,
                    match_type: None,
                    fuzzy_similarity: None,
                    edit_distance: None,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn normalize_scope(scope: &str) -> String {
    scope
        .replace('\\', "/")
        .trim_matches('*')
        .trim()
        .to_string()
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
