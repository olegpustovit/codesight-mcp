//! Integration tests: agent v1 doc summarize.

use codesight_retrieval::doc_summarizer::{
    DocSummarizerConfig, get_provider_name, heading_summary, parse_response, summarize_sections,
    title_fallback,
};
use codesight_retrieval::sections::Section;

fn make_section(heading: &str, level: u8) -> Section {
    Section {
        id: String::new(),
        file_path: String::new(),
        heading: heading.to_string(),
        heading_path: String::new(),
        level,
        start_line: 0,
        end_line: 0,
        start_byte: 0,
        end_byte: 0,
        content_hash: String::new(),
        summary: String::new(),
    }
}

// ── heading_summary ──────────────────────────────────────────────────────────
/// heading_summary must return the section title as the summary.
#[test]
fn heading_summary_returns_title() {
    let result = heading_summary("Installation Guide", 2);
    assert_eq!(result, "Installation Guide");
}
/// heading_summary must truncate titles longer than 120 characters.
#[test]
fn heading_summary_truncates_long_title() {
    let long_title = "A".repeat(200);
    let result = heading_summary(&long_title, 2);
    assert_eq!(result.len(), 120);
}

// ── title_fallback ───────────────────────────────────────────────────────────
/// Level-1 section fallback must contain "Section" or the title.
#[test]
fn title_fallback_level_1() {
    let result = title_fallback("Overview", 1);
    assert!(
        result.contains("Section") || result.contains("Overview"),
        "expected result to contain 'Section' or 'Overview', got: {result}"
    );
}
/// Level-2 section fallback must contain "Subsection" or the title.
#[test]
fn title_fallback_level_2() {
    let result = title_fallback("Install", 2);
    assert!(
        result.contains("Subsection") || result.contains("Install"),
        "expected result to contain 'Subsection' or 'Install', got: {result}"
    );
}

// ── _parse_response ──────────────────────────────────────────────────────────
/// Numbered entries are parsed into an ordered list.
#[test]
fn parse_response_basic() {
    let result = parse_response("1. Explains installation.\n2. Covers configuration.", 2);
    assert_eq!(
        result,
        vec!["Explains installation.", "Covers configuration."]
    );
}
/// Lines like "e.g., something" or "v1.2.3" must not corrupt output.
#[test]
fn parse_response_dotted_non_numbered_ignored() {
    let result = parse_response(
        "e.g., some context\nv1.2.3 released\n1. Real summary here.",
        1,
    );
    assert_eq!(result, vec!["Real summary here."]);
}
/// Out-of-range numbered entries produce empty defaults.
#[test]
fn parse_response_out_of_range_ignored() {
    let result = parse_response("5. Out of range summary.", 2);
    assert_eq!(result, vec!["", ""]);
}
/// Missing entries leave empty strings in the result.
#[test]
fn parse_response_partial() {
    let result = parse_response("1. First summary.", 3);
    assert_eq!(result, vec!["First summary.", "", ""]);
}

// ── No-AI mode ───────────────────────────────────────────────────────────────
/// summarize_sections(use_ai=false) must fill summaries using heading fallback.
#[test]
fn no_ai_mode_uses_heading_fallback() {
    let mut sections = vec![
        make_section("Overview", 1),
        make_section("Installation", 2),
        make_section("Usage", 2),
    ];
    summarize_sections(&mut sections, false);
    for sec in &sections {
        assert!(
            !sec.summary.is_empty(),
            "section '{}' should have a non-empty summary",
            sec.heading
        );
    }
}

// ── Summaries filled ─────────────────────────────────────────────────────────
/// All sections get non-empty summaries after summarize_sections.
#[test]
fn summaries_filled_for_all_sections() {
    let mut sections: Vec<Section> = (0..5)
        .map(|i| make_section(&format!("Section {i}"), 2))
        .collect();
    summarize_sections(&mut sections, false);
    assert!(sections.iter().all(|s| !s.summary.is_empty()));
}
/// summarize_sections must not clear an existing summary.
#[test]
fn preserves_existing_summary() {
    let mut sec = make_section("Intro", 1);
    sec.summary = "Already summarized.".to_string();
    let mut sections = vec![sec];
    summarize_sections(&mut sections, false);
    assert!(
        !sections[0].summary.is_empty(),
        "existing summary must not be cleared"
    );
    assert_eq!(sections[0].summary, "Already summarized.");
}

// ── Provider name detection (doc summarizer) ─────────────────────────────────
/// Explicit provider via CODESIGHT_DOC_SUMMARIZER_PROVIDER overrides auto-detect.
#[test]
fn doc_provider_name_explicit_values() {
    for provider in &["anthropic", "gemini", "openai", "minimax", "glm"] {
        let config = DocSummarizerConfig {
            explicit_provider: Some(provider.to_string()),
            ..Default::default()
        };
        assert_eq!(
            get_provider_name(&config),
            Some(provider.to_string()),
            "explicit provider '{provider}' should be returned"
        );
    }
}
/// Explicit "none" disables all providers.
#[test]
fn doc_provider_name_none_disables() {
    let config = DocSummarizerConfig {
        explicit_provider: Some("none".to_string()),
        anthropic_api_key: Some("sk-test".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), None);
}
/// Auto-detect: Anthropic takes highest priority.
#[test]
fn doc_provider_name_auto_detect_anthropic() {
    let config = DocSummarizerConfig {
        anthropic_api_key: Some("sk-test".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("anthropic".to_string()));
}
/// Auto-detect: Gemini is selected when Anthropic key is absent.
#[test]
fn doc_provider_name_auto_detect_gemini_fallback() {
    let config = DocSummarizerConfig {
        google_api_key: Some("test-key".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("gemini".to_string()));
}
/// Auto-detect: MiniMax when higher-priority keys are absent.
#[test]
fn doc_provider_name_auto_detect_minimax() {
    let config = DocSummarizerConfig {
        minimax_api_key: Some("test-key".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("minimax".to_string()));
}
/// Auto-detect: GLM when it is the only configured provider.
#[test]
fn doc_provider_name_auto_detect_glm() {
    let config = DocSummarizerConfig {
        zhipuai_api_key: Some("test-key".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("glm".to_string()));
}
/// No API keys configured returns None.
#[test]
fn doc_provider_name_no_keys_returns_none() {
    let config = DocSummarizerConfig::default();
    assert_eq!(get_provider_name(&config), None);
}
/// Explicit provider overrides auto-detect even when higher-priority keys are set.
#[test]
fn doc_provider_name_explicit_overrides_auto() {
    let config = DocSummarizerConfig {
        explicit_provider: Some("glm".to_string()),
        anthropic_api_key: Some("sk-test".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("glm".to_string()));
}
/// Unknown explicit provider falls through to auto-detection.
#[test]
fn doc_provider_name_unknown_falls_through() {
    let config = DocSummarizerConfig {
        explicit_provider: Some("unknown-provider".to_string()),
        anthropic_api_key: Some("sk-test".to_string()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("anthropic".to_string()));
}
