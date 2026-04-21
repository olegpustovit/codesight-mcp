//! Lua language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::LUA_MANIFEST;
use crate::manifest::{GrammarSpec, LUA, LanguageId, LanguageManifest};
use crate::traits::Language;

pub struct Lua;

fn load() -> TsLanguage {
    tree_sitter_lua::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "lua",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for Lua {
    fn id(&self) -> LanguageId {
        LUA
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &LUA_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_restricted(&self, _src: &str, name: &str) -> bool {
        name.starts_with('_')
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.1)
    }
}
