//! dbt framework context provider.
//!
//! Detects dbt projects via `dbt_project.yml`, parses doc blocks from `.md`
//! files, resolves `{{ doc('...') }}` references, reads YAML schema files,
//! and provides `FileContext` for model SQL files.

mod docs;
mod project;
mod provider;
mod schema;

pub use docs::{
    parse_doc_blocks, parse_doc_blocks_from_content, parse_doc_blocks_multi, resolve_description,
};
pub use project::detect_dbt_project;
pub use provider::{DbtContextProvider, DbtStats};
pub use schema::{DbtModelMetadata, parse_yml_content, parse_yml_files};
