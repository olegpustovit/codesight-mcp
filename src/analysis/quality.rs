//! Domain-level quality payload builders used by handlers and slice tests.

use serde_json::{Value, json};

use crate::analysis::complexity::{complexity_assessment, cyclomatic_complexity};
use crate::storage::{IndexStore, doc_index::DocStore};

pub fn complexity_payload(store: &IndexStore, symbol_id: &str) -> anyhow::Result<Option<Value>> {
    let Some(rec) = store.symbol_record(symbol_id)? else {
        return Ok(None);
    };

    let lines = rec
        .end_line
        .saturating_sub(rec.start_line)
        .saturating_add(1);
    let cyclomatic = cyclomatic_complexity(&rec.stored_source);
    let branch_hints = cyclomatic.saturating_sub(1);
    let first_line = rec.stored_source.lines().next().unwrap_or("");
    let param_count = count_params(first_line);
    let max_nesting = max_nesting_depth(&rec.stored_source);
    let assessment = complexity_assessment(cyclomatic);
    let repo_path = rec.root_path.to_string_lossy().to_string();

    Ok(Some(json!({
        "analysis": "complexity",
        "repo": repo_path,
        "file": rec.file_path,
        "line": rec.start_line,
        "findings": [{
            "finding_id": format!("complexity:{symbol_id}"),
            "analysis": "complexity",
            "severity": assessment,
            "confidence": 0.5,
            "summary": format!("cyclomatic {cyclomatic}, nesting {max_nesting}, {lines} lines"),
            "evidence": json!({
                "lines": lines,
                "branch_token_hints": branch_hints,
                "param_count": param_count,
                "max_nesting": max_nesting,
                "cyclomatic": cyclomatic,
            }),
            "related_refs": [symbol_id],
        }],
    })))
}

pub fn hotspots_payload(
    store: &IndexStore,
    min_complexity: u64,
    top_n: usize,
) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_for_analysis()?;
    let root_path = store.root_path().ok();
    let git_available = root_path
        .as_ref()
        .map(|root| root.join(".git").exists())
        .unwrap_or(false);

    let mut file_churn = std::collections::HashMap::<String, u64>::new();
    if git_available
        && let Some(ref root) = root_path
        && let Ok(output) = std::process::Command::new("git")
            .args(["log", "--format=", "--name-only"])
            .current_dir(root)
            .output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                *file_churn.entry(trimmed.to_string()).or_default() += 1;
            }
        }
    }

    let mut hotspots = Vec::new();
    for symbol in &all_symbols {
        let kind = symbol.kind.to_ascii_lowercase();
        if kind != "function" && kind != "method" {
            continue;
        }

        let cyclomatic = cyclomatic_complexity(&symbol.source_text) as u64;
        if cyclomatic < min_complexity {
            continue;
        }

        let churn = file_churn.get(&symbol.file_path).copied().unwrap_or(0);
        let hotspot_score = cyclomatic as f64 * (1.0 + churn as f64).ln();
        let assessment = if !git_available || churn == 0 {
            "low"
        } else if hotspot_score > 50.0 {
            "high"
        } else if hotspot_score > 20.0 {
            "medium"
        } else {
            "low"
        };

        hotspots.push(json!({
            "symbol_id": symbol.id,
            "name": symbol.name,
            "file": symbol.file_path,
            "line": symbol.start_line,
            "cyclomatic": cyclomatic,
            "churn": churn,
            "hotspot_score": hotspot_score,
            "assessment": assessment,
        }));
    }

    hotspots.sort_by(|left, right| {
        let left_score = left["hotspot_score"].as_f64().unwrap_or(0.0);
        let right_score = right["hotspot_score"].as_f64().unwrap_or(0.0);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hotspots.truncate(top_n);

    Ok(json!({
        "analysis": "hotspots",
        "hotspots": hotspots,
        "git_available": git_available,
    }))
}

pub fn doc_coverage_payload(doc_store: &DocStore, symbol_ids: &[String]) -> anyhow::Result<Value> {
    if symbol_ids.is_empty() {
        return Ok(json!({
            "analysis": "doc_coverage",
            "symbols_checked": 0,
            "documented_count": 0,
            "undocumented_count": 0,
            "coverage_pct": 0.0,
            "documented": [],
            "undocumented": [],
        }));
    }

    let all_sections = doc_store.all_sections()?;
    let mut heading_sections = std::collections::HashMap::<String, Vec<Value>>::new();
    for section in &all_sections {
        heading_sections
            .entry(section.heading.to_lowercase())
            .or_default()
            .push(json!({
                "section_id": section.id,
                "section_title": section.heading,
                "doc_path": section.file_path,
            }));
    }

    let mut documented = Vec::new();
    let mut undocumented = Vec::new();
    for symbol_id in symbol_ids {
        let symbol_name = extract_symbol_name_from_id(symbol_id);
        let heading_key = symbol_name.to_lowercase();
        if let Some(sections) = heading_sections.get(&heading_key) {
            documented.push(json!({
                "symbol_id": symbol_id,
                "symbol_name": symbol_name,
                "matching_sections": sections,
            }));
        } else {
            undocumented.push(json!({
                "symbol_id": symbol_id,
                "symbol_name": symbol_name,
            }));
        }
    }

    let documented_count = documented.len();
    let total = symbol_ids.len();
    let coverage_pct = if total == 0 {
        0.0
    } else {
        documented_count as f64 / total as f64 * 100.0
    };

    let summary = doc_store.doc_summary()?;

    Ok(json!({
        "analysis": "doc_coverage",
        "repo": summary["source_id"],
        "symbols_checked": total,
        "documented_count": documented_count,
        "undocumented_count": undocumented.len(),
        "coverage_pct": coverage_pct,
        "documented": documented,
        "undocumented": undocumented,
    }))
}

pub fn extract_symbol_name_from_id(id: &str) -> String {
    let after_colons = id.rsplit("::").next().unwrap_or(id);
    after_colons
        .split('#')
        .next()
        .unwrap_or(after_colons)
        .to_string()
}

/// Count parameters in a function signature while ignoring nested generic/composite commas.
fn count_params(signature: &str) -> usize {
    let Some(open) = signature.find('(') else {
        return 0;
    };
    let after_open = open + 1;
    let bytes = signature.as_bytes();
    let mut depth: i32 = 1;
    let mut close = signature.len();
    for (i, &byte) in bytes.iter().enumerate().skip(after_open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = signature[after_open..close].trim();
    if inner.is_empty() || inner == "void" {
        return 0;
    }

    let mut params = 1usize;
    let mut bracket_depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '<' | '[' | '(' => bracket_depth += 1,
            '>' | ']' | ')' => bracket_depth -= 1,
            ',' if bracket_depth == 0 => params += 1,
            _ => {}
        }
    }
    params
}

/// Estimate nesting depth from braces first, then indentation for Python-like bodies.
fn max_nesting_depth(body: &str) -> usize {
    let mut max_depth = 0usize;
    let mut depth = 0usize;
    for ch in body.chars() {
        match ch {
            '{' => {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if max_depth > 0 {
        return max_depth;
    }

    let mut base_indent: Option<usize> = None;
    let mut max_indent_depth = 0usize;
    let mut skipped_signature = false;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !skipped_signature {
            skipped_signature = true;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let base = *base_indent.get_or_insert(indent);
        max_indent_depth = max_indent_depth.max(indent.saturating_sub(base) / 4);
    }
    max_indent_depth
}

#[cfg(test)]
mod tests {
    use super::{count_params, extract_symbol_name_from_id, max_nesting_depth};

    #[test]
    fn extracts_symbol_name_from_stable_id() {
        assert_eq!(
            extract_symbol_name_from_id("repo::src/lib.py::helper#function"),
            "helper"
        );
        assert_eq!(extract_symbol_name_from_id("Usage"), "Usage");
    }

    #[test]
    fn counts_nested_signature_params() {
        assert_eq!(
            count_params("fn run(a: Vec<(u8, u8)>, b: Option<String>)"),
            2
        );
        assert_eq!(count_params("void thing(void)"), 0);
    }

    #[test]
    fn computes_indentation_nesting_when_no_braces() {
        let body = "def run(x):\n    if x:\n        return 1\n    return 0\n";
        assert_eq!(max_nesting_depth(body), 1);
    }
}
