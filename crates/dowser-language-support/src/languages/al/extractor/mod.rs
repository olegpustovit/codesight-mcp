//! Microsoft Dynamics 365 **AL** symbols: no dedicated query pack yet, so extraction
//! follows AL brace/keyword structure with line-oriented rules instead of shared queries.

use rustc_hash::FxHashMap;
mod helpers;
mod re;

use dowser_core::StoredSymbol;
use dowser_core::symbols::SymbolKind;
use helpers::{make_symbol, make_symbol_with_sig, strip_xml_tags};

pub struct AlExtractor;

impl AlExtractor {
    pub fn language_name(&self) -> &'static str {
        "al"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["al"]
    }

    pub fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
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
        let mut id_counts = FxHashMap::<String, u32>::default();
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
