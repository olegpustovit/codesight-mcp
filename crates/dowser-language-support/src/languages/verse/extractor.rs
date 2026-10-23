//! **Verse** (UEFN): domain-specific syntax and naming; no shared multi-language query pack covers
//! this surface, so extraction remains a dedicated override.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use dowser_core::StoredSymbol;
use dowser_core::symbols::{ExtractionTier, SymbolKind};

pub struct VerseExtractor;

impl VerseExtractor {
    pub fn language_name(&self) -> &'static str {
        "verse"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["verse"]
    }

    pub fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = source.lines().collect();
        let mut byte_offset = 0usize;
        // Track scope stack: (indent_level, name)
        let mut scope_stack: Vec<(usize, String)> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            let line_num = (i + 1) as u32;
            let line_start = byte_offset;
            let line_end = byte_offset + line.len();
            let trimmed = line.trim();

            // Skip blank lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                byte_offset += line.len() + 1;
                continue;
            }

            let indent = line.len() - line.trim_start().len();

            // Pop scope stack to match current indent
            while let Some(&(level, _)) = scope_stack.last() {
                if indent <= level {
                    scope_stack.pop();
                } else {
                    break;
                }
            }

            let scopes: Vec<String> = scope_stack.iter().map(|(_, n)| n.clone()).collect();

            // Extract docstring from preceding comment lines
            let doc = extract_docstring(&lines, i);

            // Module: NAME<...> := module:
            if let Some(caps) = re::MODULE.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                symbols.push(make_symbol(
                    file_path,
                    name,
                    SymbolKind::Class,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    doc,
                ));
                scope_stack.push((indent, name.to_string()));
            }
            // Class: name<...> := class<...>(...):
            else if let Some(caps) = re::CLASS.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                symbols.push(make_symbol(
                    file_path,
                    name,
                    SymbolKind::Class,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    doc,
                ));
                scope_stack.push((indent, name.to_string()));
            }
            // Enum: NAME<...> := enum:
            else if let Some(caps) = re::ENUM.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                symbols.push(make_symbol(
                    file_path,
                    name,
                    SymbolKind::Type,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    doc,
                ));
                scope_stack.push((indent, name.to_string()));
            }
            // Extension method: (Param:Type).MethodName<...>(...):ReturnType = ...
            else if let Some(caps) = re::EXTENSION_METHOD.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                symbols.push(make_symbol(
                    file_path,
                    name,
                    SymbolKind::Method,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                    &scopes,
                    doc,
                ));
            }
            // Method: Name<...>(...):ReturnType = ...  (inside a class scope)
            else if let Some(caps) = re::METHOD.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                let kind = if scope_stack.is_empty() {
                    SymbolKind::Function
                } else {
                    SymbolKind::Method
                };
                symbols.push(make_symbol(
                    file_path, name, kind, line_num, trimmed, line_start, line_end, source,
                    &scopes, doc,
                ));
            }
            // Variable/constant: var? Name<...>:Type = ...
            else if let Some(caps) = re::VARIABLE.captures(trimmed) {
                let Some(name) = caps.get(1).map(|g| g.as_str()) else {
                    byte_offset += line.len() + 1;
                    continue;
                };
                symbols.push(make_symbol(
                    file_path,
                    name,
                    SymbolKind::Constant,
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

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

fn extract_docstring(lines: &[&str], line_idx: usize) -> Option<String> {
    if line_idx == 0 {
        return None;
    }
    let mut comments = Vec::new();
    for i in (0..line_idx).rev() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with('#') {
            comments.push(trimmed.trim_start_matches('#').trim());
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

mod re {
    use super::*;

    // Module: NAME<visibility> := module:
    pub static MODULE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(\w+)(?:<[^>]*>)?\s*:=\s*module\s*:").expect("infallible: verse module regex")
    });

    // Class: name<visibility><specifiers> := class<specifiers>(...):
    pub static CLASS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(\w+)(?:<[^>]*>)*\s*:=\s*class(?:<[^>]*>)*\s*\(")
            .expect("infallible: verse class regex")
    });

    // Enum: NAME<visibility> := enum:
    pub static ENUM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(\w+)(?:<[^>]*>)?\s*:=\s*enum\s*:").expect("infallible: verse enum regex")
    });

    // Extension method: (Param:Type).MethodName<...>(...):ReturnType = ...
    pub static EXTENSION_METHOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\([^)]+\)\.(\w+)(?:<[^>]*>)*\s*\(")
            .expect("infallible: verse extension method regex")
    });

    // Method/function: Name<visibility>(args):ReturnType = ...
    pub static METHOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(\w+)(?:<[^>]*>)*\s*\([^)]*\)(?:<[^>]*>)*\s*:")
            .expect("infallible: verse method regex")
    });

    // Variable: var? Name<visibility>:Type = ...
    pub static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:var\s+)?(\w+)(?:<[^>]*>)\s*:\s*\w+")
            .expect("infallible: verse variable regex")
    });
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
        format!("{}.{}", scopes.join("."), name)
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
        language: dowser_core::VERSE,
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

#[cfg(test)]
mod tests {
    use super::VerseExtractor;
    use dowser_core::symbols::SymbolKind;

    const VERSE_SOURCE: &str = "\
# Module import path: /Fortnite.com/UI
UI<public> := module:

    # Base UI element
    widget<public><abstract> := class<concrete>():
        # Get the widget's visibility
        GetVisibility<public>()<transacts>:widget_visibility = external {}

        var Opacity<public>:float = external {}

    # Extension method
    (W:widget).SetVisible<public>(Visible:logic)<transacts>:void = external {}

    EWidgetColor<public> := enum:
        Red
        Green
        Blue
";

    #[test]
    fn verse_extracts_all_symbols() {
        let syms = VerseExtractor
            .extract("Fortnite.digest.verse", VERSE_SOURCE)
            .unwrap();
        let names: Vec<(&str, &str)> = syms
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();

        assert!(
            syms.iter()
                .any(|s| s.name == "UI" && s.kind == SymbolKind::Class),
            "missing UI: {names:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "widget" && s.kind == SymbolKind::Class),
            "missing widget: {names:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "GetVisibility" && s.kind == SymbolKind::Method),
            "missing GetVisibility: {names:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "Opacity" && s.kind == SymbolKind::Constant),
            "missing Opacity: {names:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "SetVisible" && s.kind == SymbolKind::Method),
            "missing SetVisible: {names:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "EWidgetColor" && s.kind == SymbolKind::Type),
            "missing EWidgetColor: {names:?}"
        );
    }

    #[test]
    fn verse_utf8_byte_offsets() {
        // U+2019 RIGHT SINGLE QUOTATION MARK = 3 bytes in UTF-8
        let source = "# Widget\u{2019}s tooltip\nMyClass<public> := class<concrete>():\n    Init<public>():void = external {}\n";
        let syms = VerseExtractor.extract("test.verse", source).unwrap();
        let cls = syms
            .iter()
            .find(|s| s.name == "MyClass")
            .expect("missing MyClass");
        let source_bytes = source.as_bytes();
        assert!(cls.start_byte < source_bytes.len());
        assert!(cls.end_byte <= source_bytes.len());
        let chunk = &source_bytes[cls.start_byte..cls.start_byte + 7];
        assert_eq!(chunk, b"MyClass");
    }
}
