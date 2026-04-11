//! Documentation section summarization: heading-based fallback summaries,
//! AI response parsing, provider detection, and batch summarization.

use super::sections::Section;

/// Maximum length for heading summaries.
const MAX_SUMMARY_LEN: usize = 120;

/// Known AI provider names for auto-detection.
const KNOWN_PROVIDERS: &[&str] = &[
    "anthropic",
    "gemini",
    "openai",
    "minimax",
    "glm",
    "openrouter",
];

/// Returns the section heading as a summary, truncated to 120 characters.
pub fn heading_summary(title: &str, _level: u32) -> String {
    if title.len() <= MAX_SUMMARY_LEN {
        title.to_string()
    } else {
        title[..MAX_SUMMARY_LEN].to_string()
    }
}

/// Returns a level-appropriate fallback summary string.
pub fn title_fallback(title: &str, level: u32) -> String {
    match level {
        1 => format!("Section: {title}"),
        2 => format!("Subsection: {title}"),
        _ => format!("Section (h{level}): {title}"),
    }
}

/// Parses numbered lines from an AI response into a fixed-size vector.
///
/// Lines matching `^(\d+)\. (.*)` are extracted; the digit must be the line
/// prefix (so "e.g., something" or "v1.2.3" do not match). Out-of-range
/// numbers are silently ignored. Missing entries are filled with `""`.
pub fn parse_response(text: &str, count: usize) -> Vec<String> {
    let mut result = vec![String::new(); count];

    for line in text.lines() {
        let trimmed = line.trim();
        // Find the position of the first ". " — the prefix must be all digits.
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty()
                && prefix.chars().all(|c| c.is_ascii_digit())
                && let Ok(num) = prefix.parse::<usize>()
                && num >= 1
                && num <= count
            {
                result[num - 1] = trimmed[dot_pos + 2..].to_string();
            }
        }
    }

    result
}

/// Configuration for the documentation summarizer's AI provider.
///
/// Mirrors the code summarizer config but reads `CODESIGHT_DOC_SUMMARIZER_PROVIDER`.
#[derive(Debug, Clone, Default)]
pub struct DocSummarizerConfig {
    pub explicit_provider: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub openai_api_base: Option<String>,
    pub minimax_api_key: Option<String>,
    pub zhipuai_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
}

/// Returns the active provider name, or `None` if summarization is disabled.
///
/// Resolution order:
/// 1. Explicit `"none"` -> `None`
/// 2. Explicit known provider -> `Some(provider)`
/// 3. Unknown explicit -> fall through to auto-detect
/// 4. Auto-detect priority: anthropic -> gemini -> openai -> minimax -> glm -> openrouter
/// 5. No keys -> `None`
pub fn get_provider_name(config: &DocSummarizerConfig) -> Option<String> {
    // Check explicit provider first.
    if let Some(ref explicit) = config.explicit_provider {
        let lower = explicit.to_lowercase();
        if lower == "none" {
            return None;
        }
        if KNOWN_PROVIDERS.contains(&lower.as_str()) {
            return Some(lower);
        }
        // Unknown explicit provider: fall through to auto-detect.
    }

    // Auto-detect by key priority.
    if config.anthropic_api_key.is_some() {
        return Some("anthropic".to_string());
    }
    if config.google_api_key.is_some() {
        return Some("gemini".to_string());
    }
    if config.openai_api_base.is_some() {
        return Some("openai".to_string());
    }
    if config.minimax_api_key.is_some() {
        return Some("minimax".to_string());
    }
    if config.zhipuai_api_key.is_some() {
        return Some("glm".to_string());
    }
    if config.openrouter_api_key.is_some() {
        return Some("openrouter".to_string());
    }

    None
}

/// Fills summaries for all sections.
///
/// When `use_ai` is false, summaries are filled using `heading_summary` (if the
/// heading is non-empty) or `title_fallback`. Existing non-empty summaries are
/// preserved.
pub fn summarize_sections(sections: &mut [Section], use_ai: bool) {
    if use_ai {
        // AI summarization is not yet implemented; fall back to heading mode.
    }

    for section in sections.iter_mut() {
        if !section.summary.is_empty() {
            continue;
        }
        if !section.heading.is_empty() {
            section.summary = heading_summary(&section.heading, section.level as u32);
        } else {
            section.summary = title_fallback(&section.heading, section.level as u32);
        }
    }
}
