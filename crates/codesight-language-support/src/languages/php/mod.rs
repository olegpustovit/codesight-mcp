//! PHP language + Laravel framework providers.

pub mod fqn;
pub mod laravel;
mod simple_framework;

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use serde_json::Value;
use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::PHP_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, PHP};
use crate::signature_density::{
    extract_return_type_after_params, find_matching_delimiter, split_top_level,
};
use crate::traits::Language;
use crate::visibility::cfamily_is_restricted;

/// PHP-specific signature density: `Type $name` parameter shape + `: Return`
/// suffix. Distinct from colon-after-name (Rust/TS/Kotlin) and
/// type-before-name (Java/C#/C++) families because PHP uses `$name` markers.
fn measure_php_signature(lang: &Php, source: &str) -> Option<f64> {
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
            score_sum += lang.type_token_score(&tokens[..var_pos].join(" "));
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, lang) {
        score_sum += lang.type_token_score(return_ty);
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
        cfamily_is_restricted(source_text)
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
        measure_php_signature(self, source)
    }
    fn uses_colon_return_suffix(&self) -> bool {
        true
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["mixed"]
    }
    fn namespace_map(&self, root: &Path) -> FxHashMap<String, String> {
        read_composer_psr4(root).unwrap_or_default()
    }
    fn pruned_dirs(&self) -> &'static [&'static str] {
        &["vendor"]
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "abstract",
            "and",
            "array",
            "as",
            "break",
            "callable",
            "case",
            "catch",
            "class",
            "clone",
            "const",
            "continue",
            "declare",
            "default",
            "do",
            "echo",
            "else",
            "elseif",
            "empty",
            "enddeclare",
            "endfor",
            "endforeach",
            "endif",
            "endswitch",
            "endwhile",
            "extends",
            "final",
            "finally",
            "fn",
            "for",
            "foreach",
            "function",
            "global",
            "goto",
            "if",
            "implements",
            "include",
            "include_once",
            "instanceof",
            "insteadof",
            "interface",
            "isset",
            "list",
            "match",
            "namespace",
            "new",
            "or",
            "print",
            "private",
            "protected",
            "public",
            "readonly",
            "require",
            "require_once",
            "return",
            "static",
            "switch",
            "throw",
            "trait",
            "try",
            "unset",
            "use",
            "var",
            "while",
            "xor",
            "yield",
            "true",
            "false",
            "null",
        ]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "__construct",
            "__destruct",
            "__toString",
            "__invoke",
            "__get",
            "__set",
            "__isset",
            "__unset",
            "__call",
            "__callStatic",
            "__clone",
            "__sleep",
            "__wakeup",
            "__serialize",
            "__unserialize",
            "jsonSerialize",
            "offsetGet",
            "offsetSet",
            "offsetExists",
            "offsetUnset",
            "count",
            "current",
            "key",
            "next",
            "rewind",
            "valid",
            "getIterator",
            "handle",
            "up",
            "down",
            "setUp",
            "tearDown",
            "boot",
            "register",
        ]
    }
    fn simple_framework_profile(
        &self,
        root: &Path,
    ) -> Option<crate::frameworks::profiles::FrameworkProfile> {
        simple_framework::detect(root)
    }
    fn resolve_fqn(
        &self,
        fqn: &str,
        root: &Path,
        source_files: &FxHashSet<String>,
    ) -> Option<String> {
        let map = self.namespace_map(root);
        if map.is_empty() {
            return None;
        }
        fqn::fqn_to_symbol(fqn, &map, source_files)
    }
}

fn read_composer_psr4(root: &Path) -> Option<FxHashMap<String, String>> {
    let content = std::fs::read_to_string(root.join("composer.json")).ok()?;
    let parsed: Value = serde_json::from_str(&content).ok()?;
    let autoload = parsed.get("autoload")?.get("psr-4")?.as_object()?;
    let mut map = FxHashMap::default();
    for (ns, dir) in autoload {
        if let Some(d) = dir.as_str() {
            map.insert(ns.clone(), d.to_string());
        }
    }
    Some(map)
}
