//! Regex and template-driven symbols for SQL where tree-sitter cannot see the construct.
//!
//! - **dbt/Jinja** — `{% macro %}`, `{% test %}`, `{% snapshot %}`, `{% materialization %}`
//!   are not SQL; they are embedded in `.sql` files and never appear in the sequel grammar.
//! - **CTE names** — when `tree-sitter-sequel` omits `common_table_expression` nodes, we match
//!   `WITH name AS (` / `, name AS (` and skip spans already emitted by the structural pass.

use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

pub(super) fn extract_ctes(
    file_path: &str,
    source: &str,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    static CTE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:WITH|,)\s+(\w+)\s+AS\s*\(").expect("infallible: sql CTE name regex")
    });

    for cap in CTE_RE.captures_iter(source) {
        let (Some(name_m), Some(full_match)) = (cap.get(1), cap.get(0)) else {
            continue;
        };
        let name = name_m.as_str();
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
            language: codesight_core::SQL,
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

struct DbtBlockSpec {
    regex: Regex,
    kind: SymbolKind,
    id_tag: &'static str,
    with_docstring: bool,
}

static DBT_BLOCK_SPECS: LazyLock<[DbtBlockSpec; 4]> = LazyLock::new(|| {
    [
        DbtBlockSpec {
            regex: Regex::new(
                r"(?s)\{%-?\s*macro\s+(\w+)\s*\(([^)]*)\)\s*-?%\}.*?\{%-?\s*endmacro\s*-?%\}",
            )
            .expect("infallible: dbt macro regex"),
            kind: SymbolKind::Function,
            id_tag: "function",
            with_docstring: true,
        },
        DbtBlockSpec {
            regex: Regex::new(
                r"(?s)\{%-?\s*test\s+(\w+)\s*\([^)]*\)\s*-?%\}.*?\{%-?\s*endtest\s*-?%\}",
            )
            .expect("infallible: dbt test regex"),
            kind: SymbolKind::Function,
            id_tag: "function",
            with_docstring: false,
        },
        DbtBlockSpec {
            regex: Regex::new(
                r"(?s)\{%-?\s*snapshot\s+(\w+)\s*-?%\}.*?\{%-?\s*endsnapshot\s*-?%\}",
            )
            .expect("infallible: dbt snapshot regex"),
            kind: SymbolKind::Type,
            id_tag: "type",
            with_docstring: false,
        },
        DbtBlockSpec {
            regex: Regex::new(
                r"(?s)\{%-?\s*materialization\s+(\w+)\s*,.*?-?%\}.*?\{%-?\s*endmaterialization\s*-?%\}",
            )
            .expect("infallible: dbt materialization regex"),
            kind: SymbolKind::Function,
            id_tag: "function",
            with_docstring: false,
        },
    ]
});

fn collect_dbt_blocks(
    spec: &DbtBlockSpec,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    for cap in spec.regex.captures_iter(source) {
        let (Some(full_match), Some(name_m)) = (cap.get(0), cap.get(1)) else {
            continue;
        };
        let name = name_m.as_str();
        let start_byte = full_match.start();
        let end_byte = full_match.end();
        let source_text = full_match.as_str().to_string();
        let sig_line = source_text.lines().next().unwrap_or("").trim().to_string();
        let summary = if spec.with_docstring {
            extract_block_comment_before(source, start_byte).unwrap_or_else(|| sig_line.clone())
        } else {
            sig_line.clone()
        };
        let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
        symbols.push(StoredSymbol {
            id: format!("{file_path}::{name}#{}", spec.id_tag),
            extraction_tier: ExtractionTier::ManualOverride,
            language: codesight_core::SQL,
            file_path: file_path.to_string(),
            qualified_name: name.to_string(),
            name: name.to_string(),
            kind: spec.kind,
            signature: sig_line,
            summary,
            start_line: source[..start_byte].lines().count() as u32,
            end_line: source[..end_byte].lines().count() as u32,
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
    for spec in DBT_BLOCK_SPECS.iter() {
        collect_dbt_blocks(spec, file_path, source, symbols)?;
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
