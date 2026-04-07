//! Code symbol summarization: docstring extraction, signature fallback,
//! provider detection, circuit breaker, and model override.

use crate::model::SymbolKind;

// ── Docstring extraction ─────────────────────────────────────────────────────

/// Extract the first sentence (ending with `.`) from a docstring,
/// considering only text before the first blank line.
///
/// - Empty or whitespace-only input returns `""`.
/// - If the first paragraph contains a period, returns text up to and
///   including that period.
/// - Otherwise returns the full first paragraph (trimmed).
pub fn extract_summary_from_docstring(docstring: &str) -> String {
    let trimmed = docstring.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Take text before the first blank line.
    let first_para = trimmed
        .split('\n')
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    // Find the first period followed by end-of-string, whitespace, or newline.
    if let Some(pos) = first_para.find('.') {
        first_para[..=pos].to_string()
    } else {
        first_para
    }
}

// ── Signature fallback ───────────────────────────────────────────────────────

/// Produce a human-readable fallback summary from a symbol's kind, name,
/// and signature when no docstring is available.
pub fn signature_fallback(kind: &str, name: &str, signature: &str) -> String {
    match kind {
        "class" => format!("Class {name}"),
        "constant" => format!("Constant {name}"),
        _ => signature.to_string(),
    }
}

// ── Simple summarize ─────────────────────────────────────────────────────────

/// Minimal symbol info used by `summarize_symbols_simple`.
#[derive(Debug, Clone)]
pub struct SummarySymbol {
    pub kind: SymbolKind,
    pub name: String,
    pub signature: String,
    pub docstring: String,
    pub summary: String,
}

/// For each symbol, set `summary` from the docstring (preferred) or
/// a signature-based fallback.
pub fn summarize_symbols_simple(symbols: &mut [SummarySymbol]) {
    for sym in symbols.iter_mut() {
        let doc = sym.docstring.trim();
        if !doc.is_empty() {
            sym.summary = extract_summary_from_docstring(&sym.docstring);
        } else {
            sym.summary = signature_fallback(sym.kind.as_str(), &sym.name, &sym.signature);
        }
    }
}

// ── Provider detection ───────────────────────────────────────────────────────

/// Configuration for the summarizer, designed for testability (no direct
/// env-var reads inside logic functions).
#[derive(Debug, Clone, Default)]
pub struct SummarizerConfig {
    pub explicit_provider: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub openai_api_base: Option<String>,
    pub minimax_api_key: Option<String>,
    pub zhipuai_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub model_name: Option<String>,
}

impl SummarizerConfig {
    /// Build a config by reading the well-known environment variables.
    pub fn from_env() -> Self {
        fn env_opt(key: &str) -> Option<String> {
            std::env::var(key).ok().filter(|v| !v.is_empty())
        }
        Self {
            explicit_provider: env_opt("CODESIGHT_SUMMARIZER_PROVIDER"),
            anthropic_api_key: env_opt("ANTHROPIC_API_KEY"),
            google_api_key: env_opt("GOOGLE_API_KEY"),
            openai_api_base: env_opt("OPENAI_API_BASE"),
            minimax_api_key: env_opt("MINIMAX_API_KEY"),
            zhipuai_api_key: env_opt("ZHIPUAI_API_KEY"),
            openrouter_api_key: env_opt("OPENROUTER_API_KEY"),
            model_name: env_opt("CODESIGHT_SUMMARIZER_MODEL"),
        }
    }
}

const KNOWN_PROVIDERS: &[&str] = &["anthropic", "gemini", "openai", "minimax", "glm"];

/// Determine the AI provider name from configuration.
///
/// - Explicit `"none"` disables all providers (returns `None`).
/// - A known explicit value is returned directly.
/// - An unknown explicit value falls through to auto-detection.
/// - Auto-detect priority: anthropic, gemini, openai, minimax, glm, openrouter.
pub fn get_provider_name(config: &SummarizerConfig) -> Option<String> {
    if let Some(ref explicit) = config.explicit_provider {
        let lower = explicit.trim().to_lowercase();
        if lower == "none" {
            return None;
        }
        if KNOWN_PROVIDERS.contains(&lower.as_str()) {
            return Some(lower);
        }
        // Unknown value — fall through to auto-detect.
    }

    // Auto-detect by key presence, in priority order.
    if config.anthropic_api_key.is_some() {
        return Some("anthropic".into());
    }
    if config.google_api_key.is_some() {
        return Some("gemini".into());
    }
    if config.openai_api_base.is_some() {
        return Some("openai".into());
    }
    if config.minimax_api_key.is_some() {
        return Some("minimax".into());
    }
    if config.zhipuai_api_key.is_some() {
        return Some("glm".into());
    }
    if config.openrouter_api_key.is_some() {
        return Some("openrouter".into());
    }

    None
}

// ── Model override ───────────────────────────────────────────────────────────

/// Return the configured model name, or `None` if empty / whitespace-only.
pub fn get_model_name(config: &SummarizerConfig) -> Option<String> {
    config
        .model_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ── Circuit breaker ──────────────────────────────────────────────────────────

/// A simple consecutive-failure circuit breaker.
///
/// After `threshold` consecutive failures, `is_open()` returns `true` and
/// callers should skip further attempts. A single `record_success()` resets
/// the counter.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    consecutive_failures: u32,
    threshold: u32,
}

impl CircuitBreaker {
    pub fn new(threshold: u32) -> Self {
        Self {
            consecutive_failures: 0,
            threshold,
        }
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    pub fn is_open(&self) -> bool {
        self.consecutive_failures >= self.threshold
    }
}
