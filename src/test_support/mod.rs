//! Synthetic workspaces and checked-in fixture paths for integration tests.
//! Used by integration tests; not part of the MCP runtime.

mod fixtures;
mod indexed;
mod repos;

pub use fixtures::doc_reference_fixtures_dir;
pub use indexed::{
    copy_fixture_tree, find_symbol_id, index_code_fixture, index_doc_fixture, make_service,
    make_service_from_config, normalize_relative_path, open_code_store, open_doc_store, repo_root,
    write_files,
};
pub use repos::{write_hierarchy_repo, write_medium_repo, write_small_repo};
