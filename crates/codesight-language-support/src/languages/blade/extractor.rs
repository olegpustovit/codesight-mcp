//! **Laravel Blade** templates mix PHP and Blade directives; compound `.blade.php` paths and
//! directive parsing are not modeled by a single SCM query file alone.
use regex::Regex;
use std::sync::LazyLock;

use codesight_core::StoredSymbol;
use codesight_core::symbols::{ExtractionTier, SymbolKind};

use crate::host_segments::{RemapOptions, WrappedSegment, extract_wrapped_host_segment};

pub struct BladeExtractor;

impl BladeExtractor {
    pub fn language_name(&self) -> &'static str {
        "blade"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        // Compound extension matched before plain `.php` in `LanguageRegistry::identify_path`.
        &["blade.php"]
    }

    pub fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = source.lines().collect();
        let mut byte_offset = 0usize;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let line_num = (i + 1) as u32;
            let line_start = byte_offset;
            let line_end = byte_offset + line.len();

            if let Some(name) = re::extends(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Type,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            } else if let Some(name) = re::section(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Method,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            } else if let Some(name) = re::component(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Class,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            } else if let Some(name) = re::include(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Function,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            } else if let Some(name) = re::push(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Constant,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            } else if let Some(name) = re::livewire(trimmed) {
                symbols.push(make_symbol(
                    file_path,
                    &name,
                    SymbolKind::Class,
                    line_num,
                    trimmed,
                    line_start,
                    line_end,
                    source,
                ));
            }

            byte_offset += line.len() + 1;
        }

        for block in blade_php_blocks(source) {
            let mut delegated = extract_wrapped_host_segment(
                "php",
                file_path,
                source,
                WrappedSegment {
                    inner_source: block.content,
                    host_start_byte: block.content_start,
                    wrapper_prefix: "<?php\n",
                    wrapper_suffix: "\n?>",
                },
                RemapOptions {
                    language_override: Some("blade"),
                    ..Default::default()
                },
            )
            .unwrap_or_default();
            delegated.retain(|symbol| {
                matches!(
                    symbol.kind,
                    SymbolKind::Class
                        | SymbolKind::Function
                        | SymbolKind::Method
                        | SymbolKind::Constant
                )
            });
            symbols.extend(delegated);
        }

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
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
) -> StoredSymbol {
    let source_text = source
        .get(start_byte..end_byte)
        .unwrap_or(line_text)
        .to_string();
    let source_hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let signature = line_text.trim().to_string();
    StoredSymbol {
        id: format!("{file_path}::{name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::BLADE,
        file_path: file_path.to_string(),
        qualified_name: name.to_string(),
        name: name.to_string(),
        kind,
        signature: signature.clone(),
        summary: signature,
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

    static EXTENDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@extends\(\s*['"]([^'"]+)['"]\s*\)"#).expect("infallible: blade regex")
    });
    static SECTION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@section\(\s*['"]([^'"]+)['"]"#).expect("infallible: blade regex")
    });
    static COMPONENT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@component\(\s*['"]([^'"]+)['"]"#).expect("infallible: blade regex")
    });
    static INCLUDE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@include\(\s*['"]([^'"]+)['"]"#).expect("infallible: blade regex")
    });
    static PUSH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@push\(\s*['"]([^'"]+)['"]"#).expect("infallible: blade regex")
    });
    static LIVEWIRE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"@livewire\(\s*['"]([^'"]+)['"]"#).expect("infallible: blade regex")
    });

    pub fn extends(s: &str) -> Option<String> {
        EXTENDS.captures(s).map(|c| c[1].to_string())
    }
    pub fn section(s: &str) -> Option<String> {
        SECTION.captures(s).map(|c| c[1].to_string())
    }
    pub fn component(s: &str) -> Option<String> {
        COMPONENT.captures(s).map(|c| c[1].to_string())
    }
    pub fn include(s: &str) -> Option<String> {
        INCLUDE.captures(s).map(|c| c[1].to_string())
    }
    pub fn push(s: &str) -> Option<String> {
        PUSH.captures(s).map(|c| c[1].to_string())
    }
    pub fn livewire(s: &str) -> Option<String> {
        LIVEWIRE.captures(s).map(|c| c[1].to_string())
    }
}

#[derive(Clone, Copy)]
struct BladePhpBlock<'a> {
    content: &'a str,
    content_start: usize,
}

fn blade_php_blocks(source: &str) -> Vec<BladePhpBlock<'_>> {
    static AT_PHP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)@php\s*(.*?)\s*@endphp").expect("infallible: blade regex")
    });
    static PHP_TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<\?php\s*(.*?)\s*\?>").expect("infallible: blade regex")
    });

    AT_PHP
        .captures_iter(source)
        .chain(PHP_TAG.captures_iter(source))
        .filter_map(|captures| {
            let content = captures.get(1)?;
            Some(BladePhpBlock {
                content: content.as_str(),
                content_start: content.start(),
            })
        })
        .collect()
}
