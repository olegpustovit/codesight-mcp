//! Thin re-export over `codesight-language-support::workspace`.
//!
//! Phase C moved `.codesight.toml` parsing and query-pack root discovery
//! into the language crate. Extractors keep the original symbol names
//! here so `registry_builder.rs` and `query_packs.rs` don't have to reach
//! into a sibling crate for config helpers.

pub use codesight_language_support::workspace::{
    project_extra_extensions, project_query_pack_roots, read_project_config,
    CodesightProjectConfig, PROJECT_CONFIG_FILE,
};
