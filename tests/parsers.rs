//! Merged parser-only test binary. See `.cursor/plans/test-suite-speedup-wave2.plan.md` Lever 3.
//! Each module is a former top-level integration test, kept verbatim under `tests/parsers/`.

#[path = "parsers/al.rs"]
mod al;
#[path = "parsers/arrow_functions.rs"]
mod arrow_functions;
#[path = "parsers/asm_language.rs"]
mod asm_language;
#[path = "parsers/blade.rs"]
mod blade;
#[path = "parsers/css.rs"]
mod css;
#[path = "parsers/dart_imports.rs"]
mod dart_imports;
#[path = "parsers/grammar_smoke.rs"]
mod grammar_smoke;
#[path = "parsers/json.rs"]
mod json;
#[path = "parsers/languages.rs"]
mod languages;
#[path = "parsers/lua.rs"]
mod lua;
#[path = "parsers/luau_language.rs"]
mod luau_language;
#[path = "parsers/razor.rs"]
mod razor;
#[path = "parsers/reference_parser.rs"]
mod reference_parser;
#[path = "parsers/sql_language.rs"]
mod sql_language;
#[path = "parsers/swift_parser.rs"]
mod swift_parser;
#[path = "parsers/synthetic_repos.rs"]
mod synthetic_repos;
#[path = "parsers/yaml_language.rs"]
mod yaml_language;
