//! Assembly **dialect-specific** extraction (GAS, NASM, ca65, WLADX): the bundle does not ship
//! one grammar that covers all targeted dialects, so this override stays outside query capture.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

pub struct AsmExtractor;

impl crate::extractors::LanguageExtractor for AsmExtractor {
    fn language_name(&self) -> &'static str {
        "asm"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["asm", "s", "65816", "inc", "z80", "spc"]
    }

    fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let dialect = detect_dialect(source, file_path);
        let mut symbols = match dialect {
            AsmDialect::WlaDx => extract_wla_dx(file_path, source)?,
            AsmDialect::Nasm => extract_nasm(file_path, source)?,
            AsmDialect::Gas => extract_gas(file_path, source)?,
            AsmDialect::Ca65 => extract_ca65(file_path, source)?,
        };
        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

#[derive(Debug)]
enum AsmDialect {
    WlaDx,
    Nasm,
    Gas,
    Ca65,
}

fn detect_dialect(source: &str, file_path: &str) -> AsmDialect {
    let lower = source.to_lowercase();
    let ext = file_path.rsplit('.').next().unwrap_or("");

    if lower.contains(".section") && (lower.contains(".define") || lower.contains(".ramsection"))
        || ext == "65816"
        || ext == "spc"
        || ext == "z80"
    {
        return AsmDialect::WlaDx;
    }
    if lower.contains(".proc") || lower.contains(".segment") {
        return AsmDialect::Ca65;
    }
    if lower.contains("%define") || lower.contains("%macro") || lower.contains("section .text") {
        return AsmDialect::Nasm;
    }
    if lower.contains(".globl")
        || lower.contains(".global")
        || (lower.contains(".text") && lower.contains(".set"))
    {
        return AsmDialect::Gas;
    }
    if ext == "s" {
        AsmDialect::Gas
    } else {
        AsmDialect::Nasm
    }
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
) -> StoredSymbol {
    let qualified_name = if scopes.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scopes.join("::"), name)
    };
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(line_text)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let summary = docstring.unwrap_or_else(|| line_text.trim().to_string());
    StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "asm".to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature: line_text.trim().to_string(),
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

fn extract_docstring(lines: &[&str], line_idx: usize) -> Option<String> {
    if line_idx == 0 {
        return None;
    }
    let mut comments = Vec::new();
    for i in (0..line_idx).rev() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with(';') {
            comments.push(trimmed.trim_start_matches(';').trim());
        } else if trimmed.is_empty() {
            if !comments.is_empty() {
                break;
            }
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

// --- WLA-DX ---
mod wla_dx_re {
    use super::*;
    static SECTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^\.section\s+"([^"]+)""#).unwrap());
    static RAMSECTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^\.ramsection\s+"([^"]+)""#).unwrap());
    static MACRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\.macro\s+(\w+)").unwrap());
    static DEFINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\.(?:define|def)\s+([\w.]+)").unwrap());
    static STRUCT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\.struct\s+(\w+)").unwrap());
    static ENUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\.enum\s+(\w+)").unwrap());
    static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([a-zA-Z][\w.]*)\s*:").unwrap());

    pub fn section(s: &str) -> Option<String> {
        SECTION.captures(s).map(|c| c[1].to_string())
    }
    pub fn ramsection(s: &str) -> Option<String> {
        RAMSECTION.captures(s).map(|c| c[1].to_string())
    }
    pub fn macro_(s: &str) -> Option<String> {
        MACRO.captures(s).map(|c| c[1].to_string())
    }
    pub fn define(s: &str) -> Option<String> {
        DEFINE.captures(s).map(|c| c[1].to_string())
    }
    pub fn struct_(s: &str) -> Option<String> {
        STRUCT.captures(s).map(|c| c[1].to_string())
    }
    pub fn enum_(s: &str) -> Option<String> {
        ENUM.captures(s).map(|c| c[1].to_string())
    }
    pub fn label(s: &str) -> Option<String> {
        LABEL.captures(s).map(|c| c[1].to_string())
    }
}

fn extract_wla_dx(file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
    let mut symbols = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut current_section: Option<String> = None;
    let mut byte_offset = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;
        let line_start = byte_offset;
        let line_end = byte_offset + line.len();

        if let Some(name) = wla_dx_re::section(trimmed) {
            let doc = extract_docstring(&lines, i);
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Class,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                doc,
            ));
            current_section = Some(name);
        } else if let Some(name) = wla_dx_re::ramsection(trimmed) {
            let doc = extract_docstring(&lines, i);
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Class,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                doc,
            ));
        } else if let Some(name) = wla_dx_re::macro_(trimmed) {
            let doc = extract_docstring(&lines, i);
            let scopes: Vec<String> = current_section.iter().cloned().collect();
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &scopes,
                doc,
            ));
        } else if let Some(name) = wla_dx_re::define(trimmed) {
            let doc = extract_docstring(&lines, i);
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                doc,
            ));
        } else if let Some(name) = wla_dx_re::struct_(trimmed) {
            let doc = extract_docstring(&lines, i);
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Type,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                doc,
            ));
        } else if let Some(name) = wla_dx_re::enum_(trimmed) {
            let doc = extract_docstring(&lines, i);
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                doc,
            ));
            // Extract enum body members until .ende
            let mut j = i + 1;
            let mut member_byte_offset = byte_offset + line.len() + 1;
            while j < lines.len() {
                let member_trimmed = lines[j].trim();
                let member_line_num = (j + 1) as u32;
                let member_line_start = member_byte_offset;
                let member_line_end = member_byte_offset + lines[j].len();
                if member_trimmed.eq_ignore_ascii_case(".ende") {
                    break;
                }
                if !member_trimmed.is_empty() && !member_trimmed.starts_with(';') {
                    // First word on the line is the member name
                    if let Some(member_name) = member_trimmed.split_whitespace().next()
                        && member_name
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                    {
                        symbols.push(make_symbol(
                            file_path,
                            member_name,
                            SymbolKind::Constant,
                            member_line_num,
                            member_trimmed,
                            member_line_start,
                            member_line_end,
                            source,
                            &[],
                            None,
                        ));
                    }
                }
                member_byte_offset += lines[j].len() + 1;
                j += 1;
            }
        } else if let Some(name) = wla_dx_re::label(trimmed) {
            let doc = extract_docstring(&lines, i);
            let scopes: Vec<String> = current_section.iter().cloned().collect();
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &scopes,
                doc,
            ));
        }

        byte_offset += line.len() + 1;
    }
    Ok(symbols)
}

// --- NASM ---
mod nasm_re {
    use super::*;
    static SECTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^section\s+(\.\w+)").unwrap());
    static DEFINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^%define\s+(\w+)").unwrap());
    static MACRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^%macro\s+(\w+)").unwrap());
    static EQU: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(\w+)\s+equ\s+").unwrap());
    static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([a-zA-Z_]\w*)\s*:").unwrap());

    pub fn section(s: &str) -> Option<String> {
        SECTION.captures(s).map(|c| c[1].to_string())
    }
    pub fn define(s: &str) -> Option<String> {
        DEFINE.captures(s).map(|c| c[1].to_string())
    }
    pub fn macro_(s: &str) -> Option<String> {
        MACRO.captures(s).map(|c| c[1].to_string())
    }
    pub fn equ(s: &str) -> Option<String> {
        EQU.captures(s).map(|c| c[1].to_string())
    }
    pub fn label(s: &str) -> Option<String> {
        LABEL.captures(s).and_then(|c| {
            let n = c[1].to_string();
            if !n.starts_with('.') { Some(n) } else { None }
        })
    }
}

fn extract_nasm(file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
    let mut symbols = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut byte_offset = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;
        let line_start = byte_offset;
        let line_end = byte_offset + line.len();

        if let Some(name) = nasm_re::section(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Class,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = nasm_re::define(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = nasm_re::macro_(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = nasm_re::equ(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = nasm_re::label(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        }

        byte_offset += line.len() + 1;
    }
    Ok(symbols)
}

// --- GAS ---
mod gas_re {
    use super::*;
    static SET_EQU: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\.(set|equ)\s+(\w+)").unwrap());
    static MACRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\.macro\s+(\w+)").unwrap());
    static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([a-zA-Z_]\w*)\s*:").unwrap());

    pub fn set_equ(s: &str) -> Option<String> {
        SET_EQU.captures(s).map(|c| c[2].to_string())
    }
    pub fn macro_(s: &str) -> Option<String> {
        MACRO.captures(s).map(|c| c[1].to_string())
    }
    pub fn label(s: &str) -> Option<String> {
        LABEL.captures(s).map(|c| c[1].to_string())
    }
}

fn extract_gas(file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
    let mut symbols = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut byte_offset = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;
        let line_start = byte_offset;
        let line_end = byte_offset + line.len();

        if let Some(name) = gas_re::set_equ(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = gas_re::macro_(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = gas_re::label(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        }

        byte_offset += line.len() + 1;
    }
    Ok(symbols)
}

// --- CA65 ---
mod ca65_re {
    use super::*;
    static SEGMENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^\.segment\s+"([^"]+)""#).unwrap());
    static PROC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\.proc\s+(\w+)").unwrap());
    static MACRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\.macro\s+(\w+)").unwrap());
    static DEFINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\.define\s+(\w+)").unwrap());
    static CONST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)\s*:?=\s*").unwrap());

    pub fn segment(s: &str) -> Option<String> {
        SEGMENT.captures(s).map(|c| c[1].to_string())
    }
    pub fn proc_(s: &str) -> Option<String> {
        PROC.captures(s).map(|c| c[1].to_string())
    }
    pub fn macro_(s: &str) -> Option<String> {
        MACRO.captures(s).map(|c| c[1].to_string())
    }
    pub fn define(s: &str) -> Option<String> {
        DEFINE.captures(s).map(|c| c[1].to_string())
    }
    pub fn const_(s: &str) -> Option<String> {
        CONST.captures(s).and_then(|c| {
            let name = c[1].to_string();
            if name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
            {
                Some(name)
            } else {
                None
            }
        })
    }
}

fn extract_ca65(file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
    let mut symbols = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut byte_offset = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;
        let line_start = byte_offset;
        let line_end = byte_offset + line.len();

        if let Some(name) = ca65_re::segment(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Class,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = ca65_re::proc_(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = ca65_re::macro_(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Function,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = ca65_re::define(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        } else if let Some(name) = ca65_re::const_(trimmed) {
            symbols.push(make_symbol(
                file_path,
                &name,
                SymbolKind::Constant,
                line_num,
                trimmed,
                line_start,
                line_end,
                source,
                &[],
                None,
            ));
        }

        byte_offset += line.len() + 1;
    }
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::AsmExtractor;
    use crate::extractors::LanguageExtractor;

    #[test]
    fn nasm_extracts_labels_and_defines() {
        let src = "section .text\n%define BUFFER_SIZE 1024\n%macro print_string 1\n    mov rsi, %1\n%endmacro\nSYS_WRITE equ 1\nprint_hello:\n    mov rax, SYS_WRITE\n    ret\n";
        let s = AsmExtractor.extract("main.asm", src).expect("asm");
        assert!(s.iter().any(|x| x.name == "BUFFER_SIZE"), "missing define");
        assert!(s.iter().any(|x| x.name == "print_string"), "missing macro");
        assert!(s.iter().any(|x| x.name == "SYS_WRITE"), "missing equ");
        assert!(s.iter().any(|x| x.name == "print_hello"), "missing label");
    }

    #[test]
    fn gas_extracts_labels_and_constants() {
        let src = ".text\n.globl main\nmain:\n    push %rbp\n    ret\nhelper:\n    ret\n.set MAX_SIZE, 100\n.equ PAGE_SIZE, 4096\n";
        let s = AsmExtractor.extract("main.s", src).expect("asm");
        assert!(s.iter().any(|x| x.name == "main"), "missing main");
        assert!(s.iter().any(|x| x.name == "helper"), "missing helper");
        assert!(s.iter().any(|x| x.name == "MAX_SIZE"), "missing .set");
        assert!(s.iter().any(|x| x.name == "PAGE_SIZE"), "missing .equ");
    }
}
