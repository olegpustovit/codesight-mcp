//! Regex and template-driven symbols for SQL where tree-sitter cannot see the construct.
//!
//! - **dbt/Jinja** — `{% macro %}`, `{% test %}`, `{% snapshot %}`, `{% materialization %}`
//!   are not SQL; they are embedded in `.sql` files and never appear in the sequel grammar.
//! - **CTE names** — when `tree-sitter-sequel` omits `common_table_expression` nodes, we match
//!   `WITH name AS (` / `, name AS (` and skip spans already emitted by the structural pass.

use anyhow::Result;
use regex::Regex;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

pub(super) fn extract_ctes(
    file_path: &str,
    source: &str,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    use std::sync::LazyLock;

    static CTE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:WITH|,)\s+(\w+)\s+AS\s*\(").unwrap());

    for cap in CTE_RE.captures_iter(source) {
        let name = cap.get(1).unwrap().as_str();
        let full_match = cap.get(0).unwrap();
        let start_byte = full_match.start();
        if symbols.iter().any(|s| s.start_byte == start_byte) {
            continue;
        }
        let end_byte = full_match.end();
        let start_line = source[..start_byte].lines().count() as u32;
        let end_line = source[..end_byte].lines().count() as u32;
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#function"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "sql".to_string(),
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            signature: sig_line.clone(),
            summary: sig_line,
            start_line,
            end_line,
            start_byte,
            end_byte,
            source_hash,
            source_text,
            file_size_bytes: source.len() as u64,
        });
    }

    Ok(())
}

pub(super) fn extract_dbt_macros(
    file_path: &str,
    source: &str,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    let macro_re =
        Regex::new(r"(?s)\{%-?\s*macro\s+(\w+)\s*\(([^)]*)\)\s*-?%\}.*?\{%-?\s*endmacro\s*-?%\}")?;
    for cap in macro_re.captures_iter(source) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let start_byte = full_match.start();
        let end_byte = full_match.end();
        let start_line = source[..start_byte].lines().count() as u32;
        let end_line = source[..end_byte].lines().count() as u32;
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let docstring = extract_block_comment_before(source, start_byte);
        let summary = docstring.unwrap_or_else(|| sig_line.clone());
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#function"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "sql".to_string(),
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            signature: sig_line,
            summary,
            start_line,
            end_line,
            start_byte,
            end_byte,
            source_hash,
            source_text,
            file_size_bytes: source.len() as u64,
        });
    }

    let test_re =
        Regex::new(r"(?s)\{%-?\s*test\s+(\w+)\s*\([^)]*\)\s*-?%\}.*?\{%-?\s*endtest\s*-?%\}")?;
    for cap in test_re.captures_iter(source) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let start_byte = full_match.start();
        let end_byte = full_match.end();
        let start_line = source[..start_byte].lines().count() as u32;
        let end_line = source[..end_byte].lines().count() as u32;
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#function"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "sql".to_string(),
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            signature: sig_line.clone(),
            summary: sig_line,
            start_line,
            end_line,
            start_byte,
            end_byte,
            source_hash,
            source_text,
            file_size_bytes: source.len() as u64,
        });
    }

    let snap_re =
        Regex::new(r"(?s)\{%-?\s*snapshot\s+(\w+)\s*-?%\}.*?\{%-?\s*endsnapshot\s*-?%\}")?;
    for cap in snap_re.captures_iter(source) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let start_byte = full_match.start();
        let end_byte = full_match.end();
        let start_line = source[..start_byte].lines().count() as u32;
        let end_line = source[..end_byte].lines().count() as u32;
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#type"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "sql".to_string(),
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Type,
            signature: sig_line.clone(),
            summary: sig_line,
            start_line,
            end_line,
            start_byte,
            end_byte,
            source_hash,
            source_text,
            file_size_bytes: source.len() as u64,
        });
    }

    let mat_re = Regex::new(
        r"(?s)\{%-?\s*materialization\s+(\w+)\s*,.*?-?%\}.*?\{%-?\s*endmaterialization\s*-?%\}",
    )?;
    for cap in mat_re.captures_iter(source) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let start_byte = full_match.start();
        let end_byte = full_match.end();
        let start_line = source[..start_byte].lines().count() as u32;
        let end_line = source[..end_byte].lines().count() as u32;
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#function"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "sql".to_string(),
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            signature: sig_line.clone(),
            summary: sig_line,
            start_line,
            end_line,
            start_byte,
            end_byte,
            source_hash,
            source_text,
            file_size_bytes: source.len() as u64,
        });
    }

    Ok(())
}

fn extract_block_comment_before(source: &str, start_byte: usize) -> Option<String> {
    let before = &source[..start_byte];
    if let Some(end) = before.rfind("#}")
        && let Some(start) = before[..end].rfind("{#")
    {
        let comment = before[start + 2..end].trim().to_string();
        if !comment.is_empty() {
            return Some(comment);
        }
    }
    let lines: Vec<&str> = before.lines().rev().collect();
    let mut comments = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            comments.push(trimmed.trim_start_matches('-').trim());
        } else if trimmed.is_empty() {
            continue;
        } else {
            break;
        }
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join(" "))
}
