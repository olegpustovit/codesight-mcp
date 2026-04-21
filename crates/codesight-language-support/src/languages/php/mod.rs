//! PHP language + Laravel framework providers.

pub mod laravel;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::PHP_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, PHP};
use crate::signature_density::{
    extract_return_type_after_params, find_matching_delimiter, split_top_level, type_token_score,
};
use crate::traits::Language;

/// PHP-specific signature density: `Type $name` parameter shape + `: Return`
/// suffix. Distinct from colon-after-name (Rust/TS/Kotlin) and
/// type-before-name (Java/C#/C++) families because PHP uses `$name` markers.
fn measure_php_signature(source: &str) -> Option<f64> {
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let params = &source[open + 1..close];
    let suffix = &source[close + 1..];

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() {
            continue;
        }
        total_slots += 1;
        let tokens: Vec<&str> = param.split_whitespace().collect();
        if let Some(var_pos) = tokens.iter().position(|token| token.starts_with('$'))
            && var_pos > 0
        {
            score_sum += type_token_score(&tokens[..var_pos].join(" "), "php");
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, "php") {
        score_sum += type_token_score(return_ty, "php");
    }

    Some(score_sum / total_slots as f64)
}

pub struct Php;

fn load() -> TsLanguage {
    tree_sitter_php::LANGUAGE_PHP.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "php",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Php {
    fn id(&self) -> LanguageId {
        PHP
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &PHP_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_restricted(&self, source_text: &str, _name: &str) -> bool {
        let trimmed = source_text.trim_start();
        if trimmed.starts_with("private ")
            || trimmed.starts_with("protected ")
            || trimmed.starts_with("internal ")
        {
            return true;
        }
        !trimmed.starts_with("public ")
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.3)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_php_signature(source)
    }
}
