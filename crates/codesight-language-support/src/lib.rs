//! codesight-language-support — every language owns its behavior through
//! `languages/<lang>/`. One crate, one trait, one registry.

pub mod complexity;
pub mod dep_graph;
pub mod extensions;
pub mod extract_dedup;
pub mod extract_error;
pub mod extract_tree_sitter;
pub mod facade;
pub mod fallback;
pub mod family;
pub mod frameworks;
pub mod generic_ast;
pub mod grammars;
pub mod host_segments;
pub mod imports_orchestration;
pub mod imports_types;
pub mod languages;
pub mod manifest;
pub mod nesting;
pub mod packages;
pub mod path_rename;
pub mod query_pack_catalog;
pub mod registry;
pub mod scm_engine;
pub mod scm_imports;
pub mod signature_density;
pub mod text_match;
pub mod traits;
pub mod visibility;
pub mod workspace;
pub mod workspace_pack;

pub use extensions::{
    DOC_EXTENSIONS, code_extensions, is_code_extension, is_doc_extension,
    is_non_code_reference_path,
};
pub use facade::{
    additional_live_roots, all_entrypoint_basenames, class_hierarchy_note_for_path,
    distinctive_symbol_kinds_for_language, fallback_type_baseline_for_path,
    function_length_warn_for_path, has_restricted_visibility, idiomatic_symbol_names_for_path,
    import_extraction_mode_for_language, is_barrel_basename, is_conventional_path,
    is_entrypoint_basename, is_outline_noise_basename, is_test_file, is_test_fixture,
    language_family_for_path, language_id_for_path, measured_type_density_for_path,
    mentions_identifier_ci_in_file, mentions_identifier_in_file, nesting_mode_for_path,
    reclassify_file_language, root_package_from_specifier, supports_import_extraction,
    test_weight_multiplier,
};
pub use fallback::{FALLBACK_LANGUAGE, FallbackLanguage};
pub use family::{LanguageFamily, NestingMode};
pub use grammars::{
    BuiltinQueryPackRow, builtin_extractor_version_stamp, builtin_grammar, builtin_grammars,
    builtin_query_packs,
};
pub use imports_types::{ImportEdge, source_slice};
pub use manifest::{
    AL, ASM, BASH, BLADE, C, CPP, CSHARP, CSS, DART, EJS, ELIXIR, ERLANG, FORTRAN, GLEAM, GO,
    GrammarSpec, HostProfile, JAVA, JAVASCRIPT, JSON, JSX, KOTLIN, LESS, LUA, LUAU, LanguageId,
    LanguageManifest, MARKDOWN, NIX, PERL, PHP, PYTHON, ProfileId, RAZOR, RUBY, RUST,
    ResolvedLanguage, SASS, SCSS, SQL, STYL, SWIFT, TOML, TREE_SITTER_QUERY, TSX, TYPESCRIPT,
    UNKNOWN_LANGUAGE, VERSE, VUE, XML, YAML,
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

/// Resolve a language-native FQN (PHP PSR-4, future Python/Java/…) against
/// the workspace by asking every registered [`Language`]'s
/// `resolve_fqn`. The first language that accepts the FQN wins.
pub fn resolve_fqn(
    fqn: &str,
    root: &std::path::Path,
    source_files: &std::collections::HashSet<String>,
) -> Option<String> {
    builtin_registry()
        .all()
        .find_map(|lang| lang.resolve_fqn(fqn, root, source_files))
}
