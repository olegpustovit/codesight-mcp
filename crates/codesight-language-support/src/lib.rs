//! codesight-language-support — every language owns its behavior through
//! `languages/<lang>/`. One crate, one trait, one registry.
#![forbid(unsafe_code)]

pub mod facade;
pub mod fallback;
pub mod family;
pub mod frameworks;
pub mod grammars;
pub mod imports_types;
pub mod languages;
pub mod manifest;
pub mod nesting;
pub mod registry;
pub mod scm_imports;
pub mod signature_density;
pub mod traits;
pub mod workspace;

pub use facade::{
    additional_live_root_files, all_entrypoint_basenames, fallback_type_baseline_for_path,
    function_length_warn_for_path, has_restricted_visibility, import_extraction_mode_for_language,
    is_conventional_path, is_entrypoint_basename, is_test_file, is_test_fixture,
    language_family_for_path, language_id_for_path, measured_type_density_for_path,
    nesting_mode_for_path, root_package_from_specifier, supports_import_extraction,
    test_weight_multiplier,
};
pub use fallback::{FALLBACK_LANGUAGE, FallbackLanguage};
pub use family::{LanguageFamily, NestingMode};
pub use imports_types::{ImportEdge, source_slice};
pub use grammars::{
    BuiltinQueryPackRow, builtin_extractor_version_stamp, builtin_grammar, builtin_grammars,
    builtin_query_packs,
};
pub use manifest::{
    AL, ASM, BASH, BLADE, C, CPP, CSHARP, CSS, DART, EJS, ELIXIR, ERLANG, FORTRAN, GLEAM, GO,
    GrammarSpec, HostProfile, JAVA, JAVASCRIPT, JSON, JSX, KOTLIN, LESS, LUA, LUAU, LanguageId,
    LanguageManifest, MARKDOWN, NIX, PERL, PHP, PYTHON, ProfileId, RAZOR, RUBY, RUST, SASS, SCSS,
    SQL, STYL, SWIFT, TOML, TREE_SITTER_QUERY, TSX, TYPESCRIPT, UNKNOWN_LANGUAGE, VERSE, VUE, XML,
    YAML, ResolvedLanguage,
};
pub use nesting::{max_nesting_depth_for_path, max_nesting_depth_with_mode};
pub use registry::LanguageRegistry;
pub use traits::{ImportExtractionMode, Language};
pub use workspace::{
    CodesightProjectConfig, PROJECT_CONFIG_FILE, project_extra_extensions,
    project_query_pack_roots, read_project_config,
};

/// Process-wide lazy registry.
pub fn builtin_registry() -> &'static LanguageRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();
    REGISTRY.get_or_init(LanguageRegistry::builtin)
}
