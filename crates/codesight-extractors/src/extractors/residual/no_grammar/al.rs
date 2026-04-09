//! Microsoft Dynamics 365 **AL** symbols: no dedicated query pack yet, so extraction
//! follows AL brace/keyword structure with line-oriented rules instead of shared queries.
use regex::Regex;
use std::sync::LazyLock;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

pub struct AlExtractor;

impl crate::extractors::LanguageExtractor for AlExtractor {
    fn language_name(&self) -> &'static str {
        "al"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["al"]
    }

    fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = source.lines().collect();
        let mut byte_offset = 0usize;
        // Stack of parent object names for qualified names
        let mut parent_stack: Vec<String> = Vec::new();
        // Track brace depth for scope
        let mut brace_depth: i32 = 0;
        // Map from brace_depth to parent name (when we entered that depth)
        let mut scope_at_depth: Vec<(i32, String)> = Vec::new();
        // Collect decorators preceding a procedure
        let mut pending_decorators: Vec<String> = Vec::new();
        // Collect doc comments preceding a symbol
        let mut pending_doc_lines: Vec<String> = Vec::new();
        // Track preceding comment lines for inline-comment docstrings
        let mut preceding_comments: Vec<String> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let line_num = (i + 1) as u32;
            let line_start = byte_offset;
            let line_end = byte_offset + line.len();

            // Collect XML doc comments (/// ...)
            if trimmed.starts_with("///") {
                let doc_text = trimmed.trim_start_matches('/').trim();
                // Strip XML tags for cleaner docstring
                let cleaned = strip_xml_tags(doc_text);
                if !cleaned.is_empty() {
                    pending_doc_lines.push(cleaned);
                }
                preceding_comments.clear();
                byte_offset += line.len() + 1;
                continue;
            }

            // Collect inline comments (// ...)
            if trimmed.starts_with("//") && !trimmed.starts_with("///") {
                let comment_text = trimmed.trim_start_matches('/').trim().to_string();
                preceding_comments.push(comment_text);
                byte_offset += line.len() + 1;
                continue;
            }

            // Collect decorators [...]
            if trimmed.starts_with('[') && trimmed.contains(']') {
                pending_decorators.push(trimmed.to_string());
                byte_offset += line.len() + 1;
                continue;
            }

            // Build docstring from pending doc lines or preceding comments
            let docstring = if !pending_doc_lines.is_empty() {
                Some(pending_doc_lines.join(" "))
            } else if !preceding_comments.is_empty() {
                Some(preceding_comments.join(" "))
            } else {
                None
            };

            // Track brace depth
            let open_braces = trimmed.chars().filter(|&c| c == '{').count() as i32;
            let close_braces = trimmed.chars().filter(|&c| c == '}').count() as i32;
            let prev_depth = brace_depth;
            brace_depth += open_braces - close_braces;

            // Pop parent scope when brace depth drops (net closing on this line)
            if brace_depth < prev_depth {
                while let Some((depth, _)) = scope_at_depth.last() {
                    if brace_depth <= *depth {
                        scope_at_depth.pop();
                        parent_stack.pop();
                    } else {
                        break;
                    }
                }
            }

            let current_parent = parent_stack.last().cloned();

            // Top-level object declarations
            if let Some((kind, name)) = re::top_level_object(trimmed) {
                let sym_kind = match kind.as_str() {
                    "enum" | "interface" => SymbolKind::Type,
                    _ => SymbolKind::Class,
                };
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    sym_kind,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &[],
                    docstring.clone(),
                    &pending_decorators,
                ));
                parent_stack.push(name.clone());
                scope_at_depth.push((brace_depth, name));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Procedures (procedure, local procedure, internal procedure)
            else if let Some(name) = re::procedure(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Method,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                    &pending_decorators,
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Triggers
            else if let Some(name) = re::trigger(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Method,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                    &pending_decorators,
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Events (in controladdin)
            else if let Some(name) = re::event(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Method,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                    &pending_decorators,
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Fields: field(N; Name; Type) or field(Name; SourceExpr) in page layout
            else if let Some((name, sig)) = re::field(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Enum values: value(N; Name)
            else if let Some((name, sig)) = re::enum_value(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Actions: action(Name)
            else if let Some((name, sig)) = re::action(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Function,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Keys: key(Name; Columns)
            else if let Some((name, sig)) = re::key(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Columns: column(Name; SourceExpr)
            else if let Some((name, sig)) = re::column(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // FieldGroups: fieldgroup(Name; Columns)
            else if let Some((name, sig)) = re::fieldgroup(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // DataItems: dataitem(Name; SourceTable)
            else if let Some((name, sig)) = re::dataitem(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Type,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                // DataItems create a scope for nested columns
                parent_stack.push(if let Some(p) = &current_parent {
                    p.to_string()
                } else {
                    name.clone()
                });
                scope_at_depth.push((brace_depth, name));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // XMLport elements: textelement, tableelement, fieldelement
            else if let Some((name, sig)) = re::xmlport_element(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Type,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            }
            // Variables: var Name: Type  (standalone var lines, also after procedure)
            else if let Some((name, sig)) = re::variable(trimmed) {
                let scopes: Vec<String> = parent_stack.clone();
                symbols.push(make_symbol_with_sig(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    &sig,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    docstring.clone(),
                ));
                pending_doc_lines.clear();
                preceding_comments.clear();
                pending_decorators.clear();
            } else {
                // Clear doc/comment state on non-matching non-empty lines
                if !trimmed.is_empty()
                    && trimmed != "var"
                    && trimmed != "begin"
                    && trimmed != "end;"
                {
                    pending_doc_lines.clear();
                    preceding_comments.clear();
                    pending_decorators.clear();
                }
            }

            byte_offset += line.len() + 1;
        }

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));

        // Deduplicate IDs by appending a counter suffix
        let mut id_counts = std::collections::HashMap::<String, u32>::new();
        for sym in &mut symbols {
            let count = id_counts.entry(sym.id.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                sym.id = format!("{}_{}", sym.id, count);
            }
        }

        Ok(symbols)
    }
}

fn strip_xml_tags(s: &str) -> String {
    static XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    XML_TAG.replace_all(s, "").trim().to_string()
}

#[allow(clippy::too_many_arguments)]
fn make_symbol(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    line_text: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    scopes: &[String],
    docstring: Option<String>,
    _decorators: &[String],
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scopes.last().unwrap(), name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(line_text)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let signature = line_text.trim().to_string();
    let summary = docstring.unwrap_or_else(|| signature.clone());
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "al".to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature,
        summary,
        start_line: line_num,
        end_line: line_num,
        start_byte,
        end_byte,
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_symbol_with_sig(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    line_num: u32,
    signature: &str,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    scopes: &[String],
    docstring: Option<String>,
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scopes.last().unwrap(), name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(signature)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let summary = docstring.unwrap_or_else(|| signature.to_string());
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "al".to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature: signature.to_string(),
        summary,
        start_line: line_num,
        end_line: line_num,
        start_byte,
        end_byte,
        source_hash,
        source_text,
        file_size_bytes: source.len() as u64,
    }
}

mod re {
    use super::*;

    // Top-level objects: table, codeunit, enum, page, report, query, xmlport, controladdin, interface
    // Also extensions: pageextension, tableextension, enumextension
    static TOP_LEVEL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)^(table|codeunit|page|report|query|xmlport|pageextension|tableextension|enumextension|enum|controladdin)\s+(?:\d+\s+)?"([^"]+)"(?:\s+extends\s+"[^"]+")?"#,
        )
        .unwrap()
    });

    // Interface: interface "Name"
    static INTERFACE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^interface\s+"([^"]+)""#).unwrap());

    // Unquoted controladdin: controladdin MyAddIn
    static CONTROLADDIN_UNQUOTED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^controladdin\s+(\w+)"#).unwrap());

    // Procedures
    static PROCEDURE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(?:local\s+|internal\s+)?procedure\s+(\w+)").unwrap());

    // Triggers
    static TRIGGER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^trigger\s+(\w+)\s*\(").unwrap());

    // Events (in controladdin)
    static EVENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^event\s+(\w+)\s*\(").unwrap());

    // Fields: field(N; Name; Type) or field(N; "Name"; Type) or field(Name; SourceExpr) in page
    static FIELD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^field\(([^)]+)\)"#).unwrap());

    // Enum values: value(N; Name)
    static VALUE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^value\(([^)]+)\)"#).unwrap());

    // Actions: action(Name) or action("Name")
    static ACTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^action\(([^)]+)\)"#).unwrap());

    // Keys: key(Name; Columns)
    static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)^key\(([^)]+)\)"#).unwrap());

    // Columns: column(Name; SourceExpr)
    static COLUMN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^column\(([^)]+)\)"#).unwrap());

    // FieldGroups: fieldgroup(Name; Columns)
    static FIELDGROUP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^fieldgroup\(([^)]+)\)"#).unwrap());

    // DataItems: dataitem(Name; SourceTable)
    static DATAITEM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^dataitem\(([^)]+)\)"#).unwrap());

    // XMLport elements: textelement(Name), tableelement(Name; Table), fieldelement(Name; SourceExpr)
    static XMLPORT_ELEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)^(textelement|tableelement|fieldelement)\(([^)]+)\)"#).unwrap()
    });

    // Variables: Name: Type (inside var block)
    static VARIABLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\w+)\s*:\s*(.+?)\s*;$").unwrap());

    pub fn top_level_object(s: &str) -> Option<(String, String)> {
        if let Some(caps) = TOP_LEVEL.captures(s) {
            return Some((caps[1].to_lowercase(), caps[2].to_string()));
        }
        if let Some(caps) = INTERFACE.captures(s) {
            return Some(("interface".to_string(), caps[1].to_string()));
        }
        if let Some(caps) = CONTROLADDIN_UNQUOTED.captures(s) {
            let name = caps[1].to_string();
            // Don't match if already matched by TOP_LEVEL (quoted version)
            if !s.contains('"') {
                return Some(("controladdin".to_string(), name));
            }
        }
        None
    }

    pub fn procedure(s: &str) -> Option<String> {
        PROCEDURE.captures(s).map(|c| c[1].to_string())
    }

    pub fn trigger(s: &str) -> Option<String> {
        TRIGGER.captures(s).map(|c| c[1].to_string())
    }

    pub fn event(s: &str) -> Option<String> {
        EVENT.captures(s).map(|c| c[1].to_string())
    }

    pub fn field(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("field(") {
            return None;
        }
        let caps = FIELD.captures(s)?;
        let inner = &caps[1];
        let name = extract_second_part(inner)?;
        let sig = format!("field({})", inner);
        Some((name, sig))
    }

    pub fn enum_value(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("value(") {
            return None;
        }
        let caps = VALUE.captures(s)?;
        let inner = &caps[1];
        let name = extract_second_part(inner)?;
        let sig = format!("value({})", inner);
        Some((name, sig))
    }

    pub fn action(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("action(") {
            return None;
        }
        let caps = ACTION.captures(s)?;
        let inner = &caps[1];
        let name = unquote(inner.trim());
        let sig = format!("action({})", inner);
        Some((name, sig))
    }

    pub fn key(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("key(") {
            return None;
        }
        let caps = KEY.captures(s)?;
        let inner = &caps[1];
        let name = extract_first_part(inner)?;
        let sig = format!("key({})", inner);
        Some((name, sig))
    }

    pub fn column(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("column(") {
            return None;
        }
        let caps = COLUMN.captures(s)?;
        let inner = &caps[1];
        let name = extract_first_part(inner)?;
        let sig = format!("column({})", inner);
        Some((name, sig))
    }

    pub fn fieldgroup(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("fieldgroup(") {
            return None;
        }
        let caps = FIELDGROUP.captures(s)?;
        let inner = &caps[1];
        let name = extract_first_part(inner)?;
        let sig = format!("fieldgroup({})", inner);
        Some((name, sig))
    }

    pub fn dataitem(s: &str) -> Option<(String, String)> {
        if !s.to_lowercase().starts_with("dataitem(") {
            return None;
        }
        let caps = DATAITEM.captures(s)?;
        let inner = &caps[1];
        let name = extract_first_part(inner)?;
        let sig = format!("dataitem({})", inner);
        Some((name, sig))
    }

    pub fn xmlport_element(s: &str) -> Option<(String, String)> {
        let caps = XMLPORT_ELEM.captures(s)?;
        let elem_type = &caps[1];
        let inner = &caps[2];
        let name = extract_first_part(inner)?;
        let sig = format!("{}({})", elem_type.to_lowercase(), inner);
        Some((name, sig))
    }

    pub fn variable(s: &str) -> Option<(String, String)> {
        let caps = VARIABLE.captures(s)?;
        let name = caps[1].to_string();
        let type_str = caps[2].to_string();
        // Skip keywords that look like variables
        let lower = name.to_lowercase();
        if lower == "begin"
            || lower == "end"
            || lower == "var"
            || lower == "if"
            || lower == "then"
            || lower == "else"
            || lower == "exit"
            || lower == "caption"
            || lower == "clustered"
            || lower == "sourcetable"
        {
            return None;
        }
        let sig = format!("{}: {}", name, type_str);
        Some((name, sig))
    }

    /// Extract the name from "N; Name" → "Name" (second semicolon-separated part)
    fn extract_second_part(inner: &str) -> Option<String> {
        let parts: Vec<&str> = inner.splitn(3, ';').collect();
        if parts.len() >= 2 {
            Some(unquote(parts[1].trim()))
        } else {
            None
        }
    }

    /// Extract the name from "Name; ..." → "Name" (first semicolon-separated part)
    fn extract_first_part(inner: &str) -> Option<String> {
        let parts: Vec<&str> = inner.splitn(2, ';').collect();
        if !parts.is_empty() {
            Some(unquote(parts[0].trim()))
        } else {
            None
        }
    }

    fn unquote(s: &str) -> String {
        let s = s.trim();
        if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            s[1..s.len() - 1].to_string()
        } else {
            s.to_string()
        }
    }
}
