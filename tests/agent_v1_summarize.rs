//! Integration tests: agent v1 summarize.

use codesight_mcp::model::SymbolKind;
use codesight_mcp::summarizer::{
    CircuitBreaker, SummarizerConfig, SummarySymbol, extract_summary_from_docstring,
    get_model_name, get_provider_name, signature_fallback, summarize_symbols_simple,
};

// ── Extract summary from docstring ───────────────────────────────────────────
/// First sentence before a blank line is the summary.
#[test]
fn extract_summary_from_docstring_first_sentence() {
    let result = extract_summary_from_docstring("Do something cool.\n\nMore details here.");
    assert_eq!(result, "Do something cool.");
}
/// Docstring without a period is returned as-is (up to first blank line).
#[test]
fn extract_summary_from_docstring_no_period() {
    let result = extract_summary_from_docstring("Do something cool");
    assert_eq!(result, "Do something cool");
}
/// Empty or whitespace-only docstring returns empty string.
#[test]
fn extract_summary_from_docstring_empty() {
    assert_eq!(extract_summary_from_docstring(""), "");
    assert_eq!(extract_summary_from_docstring("   "), "");
}
/// A multi-line docstring without a blank separator still extracts the first sentence.
#[test]
fn extract_summary_from_docstring_multiline() {
    let result = extract_summary_from_docstring("First sentence.\nSecond sentence.");
    assert_eq!(result, "First sentence.");
}

// ── Signature fallback ───────────────────────────────────────────────────────
/// Function kind: fallback is the raw signature string.
#[test]
fn signature_fallback_function() {
    let result = signature_fallback("function", "foo", "def foo(x: int) -> str:");
    assert_eq!(result, "def foo(x: int) -> str:");
}
/// Class kind: fallback is "Class {name}" (not the raw signature).
#[test]
fn signature_fallback_class() {
    let result = signature_fallback("class", "MyClass", "class MyClass(Base):");
    assert_eq!(result, "Class MyClass");
}
/// Constant kind: fallback is "Constant {name}".
#[test]
fn signature_fallback_constant() {
    let result = signature_fallback("constant", "MAX_SIZE", "MAX_SIZE = 100");
    assert_eq!(result, "Constant MAX_SIZE");
}

// ── Simple summarize ─────────────────────────────────────────────────────────
/// When a symbol has a docstring, summarize_symbols_simple uses it as the summary.
#[test]
fn simple_summarize_uses_docstring() {
    let mut symbols = vec![SummarySymbol {
        kind: SymbolKind::Function,
        name: "foo".into(),
        signature: "def foo():".into(),
        docstring: "Does something useful.".into(),
        summary: String::new(),
    }];
    summarize_symbols_simple(&mut symbols);
    assert_eq!(symbols[0].summary, "Does something useful.");
}
/// When a symbol has no docstring, summarize_symbols_simple falls back to signature.
#[test]
fn simple_summarize_fallback_to_signature() {
    let mut symbols = vec![SummarySymbol {
        kind: SymbolKind::Function,
        name: "foo".into(),
        signature: "def foo(x: int) -> str:".into(),
        docstring: String::new(),
        summary: String::new(),
    }];
    summarize_symbols_simple(&mut symbols);
    assert!(
        symbols[0].summary.contains("def foo"),
        "expected summary to contain 'def foo', got: {}",
        symbols[0].summary
    );
}

// ── Provider name detection ──────────────────────────────────────────────────
/// Explicit provider selection via env var must win over auto-detect.
#[test]
fn provider_name_explicit_values() {
    for provider in &["anthropic", "gemini", "openai", "minimax", "glm"] {
        let config = SummarizerConfig {
            explicit_provider: Some(provider.to_string()),
            ..Default::default()
        };
        assert_eq!(
            get_provider_name(&config),
            Some(provider.to_string()),
            "explicit provider '{provider}' should be returned as-is"
        );
    }
}
/// Explicit "none" must disable AI providers even when API keys are present.
#[test]
fn provider_name_none_disables() {
    let config = SummarizerConfig {
        explicit_provider: Some("none".into()),
        anthropic_api_key: Some("sk-test".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), None);
}
/// Auto-detect follows: Anthropic -> Gemini -> OpenAI -> MiniMax -> GLM -> OpenRouter.
#[test]
fn provider_name_auto_detect_priority() {
    // Set openai + minimax + glm; openai should win (highest priority of those).
    let config = SummarizerConfig {
        openai_api_base: Some("http://localhost".into()),
        minimax_api_key: Some("test".into()),
        zhipuai_api_key: Some("test".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("openai".into()));
}
/// Unknown explicit value falls through to auto-detection.
#[test]
fn provider_name_unknown_falls_back_to_auto() {
    let config = SummarizerConfig {
        explicit_provider: Some("unknown-provider".into()),
        minimax_api_key: Some("test-key".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("minimax".into()));
}
/// MiniMax is detected when higher-priority providers are absent.
#[test]
fn provider_name_auto_detect_minimax() {
    let config = SummarizerConfig {
        minimax_api_key: Some("test-key".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("minimax".into()));
}
/// GLM is detected when it is the only configured provider.
#[test]
fn provider_name_auto_detect_glm() {
    let config = SummarizerConfig {
        zhipuai_api_key: Some("test-key".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("glm".into()));
}
/// OpenRouter is detected when it is the only configured provider.
#[test]
fn provider_name_auto_detect_openrouter() {
    let config = SummarizerConfig {
        openrouter_api_key: Some("test-key".into()),
        ..Default::default()
    };
    assert_eq!(get_provider_name(&config), Some("openrouter".into()));
}

// ── Circuit breaker ──────────────────────────────────────────────────────────
/// After N consecutive failures the circuit breaker opens.
#[test]
fn circuit_breaker_opens_after_consecutive_failures() {
    let mut cb = CircuitBreaker::new(3);
    assert!(!cb.is_open(), "should start closed");

    cb.record_failure();
    assert!(!cb.is_open(), "1 failure < threshold");
    cb.record_failure();
    assert!(!cb.is_open(), "2 failures < threshold");
    cb.record_failure();
    assert!(cb.is_open(), "3 failures == threshold => open");

    // Stays open on further failures.
    cb.record_failure();
    assert!(cb.is_open());
}
/// A success after fewer than N failures resets the counter.
#[test]
fn circuit_breaker_success_resets_counter() {
    let mut cb = CircuitBreaker::new(3);

    cb.record_failure();
    cb.record_success(); // resets
    cb.record_failure();
    assert!(
        !cb.is_open(),
        "counter should have reset after success, so 1 failure < threshold"
    );

    // Need 3 consecutive to open.
    cb.record_failure();
    cb.record_failure();
    assert!(cb.is_open(), "3 consecutive failures => open");
}

// ── Model override via config ────────────────────────────────────────────────
#[test]
fn model_override_returns_configured_value() {
    let config = SummarizerConfig {
        model_name: Some("my-custom-model".into()),
        ..Default::default()
    };
    assert_eq!(get_model_name(&config), Some("my-custom-model".into()));
}
#[test]
fn model_override_none_when_empty() {
    let config = SummarizerConfig {
        model_name: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(get_model_name(&config), None);
}
#[test]
fn model_override_strips_whitespace() {
    let config = SummarizerConfig {
        model_name: Some("  claude-haiku  ".into()),
        ..Default::default()
    };
    assert_eq!(get_model_name(&config), Some("claude-haiku".into()));
}
#[test]
fn model_override_none_for_whitespace_only() {
    let config = SummarizerConfig {
        model_name: Some("   ".into()),
        ..Default::default()
    };
    assert_eq!(get_model_name(&config), None);
}
