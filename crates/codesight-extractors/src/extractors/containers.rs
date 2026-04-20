use std::sync::LazyLock;

use anyhow::{Result, anyhow};
use regex::Regex;

use crate::core::pipeline::StoredSymbol;

static BUILTIN_REGISTRY: LazyLock<super::ExtractorRegistry> =
    LazyLock::new(super::ExtractorRegistry::default);

#[derive(Clone, Copy, Debug)]
pub(crate) struct SegmentOrigin {
    pub host_start_byte: usize,
    pub host_line_offset: u32,
    pub delegated_prefix_bytes: usize,
    pub delegated_prefix_lines: u32,
}

impl SegmentOrigin {
    pub(crate) fn new(host_source: &str, host_start_byte: usize) -> Self {
        Self {
            host_start_byte,
            host_line_offset: line_offset_for_byte(host_source, host_start_byte),
            delegated_prefix_bytes: 0,
            delegated_prefix_lines: 0,
        }
    }

    pub(crate) fn with_prefix(mut self, delegated_prefix: &str) -> Self {
        self.delegated_prefix_bytes = delegated_prefix.len();
        self.delegated_prefix_lines = delegated_prefix.lines().count() as u32;
        self
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RemapOptions<'a> {
    pub language_override: Option<&'a str>,
    pub qualified_name_prefix: Option<&'a str>,
    pub strip_qualified_name_prefix: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HtmlTagBlock<'a> {
    pub attrs: &'a str,
    pub content: &'a str,
    pub content_start_byte: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WrappedSegment<'a> {
    pub inner_source: &'a str,
    pub host_start_byte: usize,
    pub wrapper_prefix: &'a str,
    pub wrapper_suffix: &'a str,
}

pub(crate) fn extract_symbols(
    language_name: &str,
    file_path: &str,
    source: &str,
) -> Result<Vec<StoredSymbol>> {
    let extractor = BUILTIN_REGISTRY
        .extractor_for_language(language_name)
        .ok_or_else(|| {
            anyhow!("no extractor registered for delegated language '{language_name}'")
        })?;
    extractor.extract(file_path, source)
}

pub(crate) fn extract_host_segment(
    language_name: &str,
    file_path: &str,
    host_source: &str,
    delegated_source: &str,
    origin: SegmentOrigin,
    options: RemapOptions<'_>,
) -> Result<Vec<StoredSymbol>> {
    let delegated = extract_symbols(language_name, file_path, delegated_source)?;
    Ok(remap_symbols(
        delegated,
        file_path,
        host_source,
        origin,
        options,
    ))
}

pub(crate) fn extract_wrapped_host_segment(
    language_name: &str,
    file_path: &str,
    host_source: &str,
    segment: WrappedSegment<'_>,
    options: RemapOptions<'_>,
) -> Result<Vec<StoredSymbol>> {
    let delegated_source = format!(
        "{}{}{}",
        segment.wrapper_prefix, segment.inner_source, segment.wrapper_suffix
    );
    let origin = SegmentOrigin::new(host_source, segment.host_start_byte)
        .with_prefix(segment.wrapper_prefix);
    extract_host_segment(
        language_name,
        file_path,
        host_source,
        &delegated_source,
        origin,
        options,
    )
}

pub(crate) fn remap_symbols(
    delegated: Vec<StoredSymbol>,
    file_path: &str,
    host_source: &str,
    origin: SegmentOrigin,
    options: RemapOptions<'_>,
) -> Vec<StoredSymbol> {
    delegated
        .into_iter()
        .map(|mut symbol| {
            let start_delta = symbol
                .start_byte
                .saturating_sub(origin.delegated_prefix_bytes);
            let end_delta = symbol
                .end_byte
                .saturating_sub(origin.delegated_prefix_bytes);
            symbol.start_byte = origin.host_start_byte + start_delta;
            symbol.end_byte = origin.host_start_byte + end_delta;
            symbol.start_line = symbol
                .start_line
                .saturating_sub(origin.delegated_prefix_lines)
                + origin.host_line_offset;
            symbol.end_line = symbol
                .end_line
                .saturating_sub(origin.delegated_prefix_lines)
                + origin.host_line_offset;

            if let Some(prefix) = options.strip_qualified_name_prefix
                && let Some(stripped) = symbol.qualified_name.strip_prefix(prefix)
            {
                symbol.qualified_name = stripped.to_string();
            }
            if let Some(prefix) = options.qualified_name_prefix {
                symbol.qualified_name = if symbol.qualified_name.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{prefix}.{}", symbol.qualified_name)
                };
            }
            if let Some(language) = options.language_override {
                symbol.language = codesight_core::LanguageId::intern(language);
            }

            symbol.file_path = file_path.to_string();
            symbol.file_size_bytes = host_source.len() as u64;
            if let Some(host_text) = host_source.get(symbol.start_byte..symbol.end_byte) {
                symbol.source_text = host_text.to_string();
                symbol.source_hash = blake3::hash(symbol.source_text.as_bytes())
                    .to_hex()
                    .to_string();
            }
            symbol.id = format!(
                "{file_path}::{}#{}",
                symbol.qualified_name,
                symbol.kind.as_str()
            );
            symbol
        })
        .collect()
}

pub(crate) fn find_html_tag_blocks<'a>(
    source: &'a str,
    tag_name: &str,
) -> Result<Vec<HtmlTagBlock<'a>>> {
    let escaped = regex::escape(tag_name);
    let pattern = format!(r"(?is)<{escaped}\b([^>]*)>(.*?)</{escaped}\s*>");
    let regex = Regex::new(&pattern)?;
    Ok(regex
        .captures_iter(source)
        .filter_map(|captures| {
            let attrs = captures.get(1)?;
            let content = captures.get(2)?;
            Some(HtmlTagBlock {
                attrs: attrs.as_str(),
                content: content.as_str(),
                content_start_byte: content.start(),
            })
        })
        .collect())
}

pub(crate) fn line_offset_for_byte(source: &str, byte_offset: usize) -> u32 {
    source[..byte_offset.min(source.len())].lines().count() as u32
}
