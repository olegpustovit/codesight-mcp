//! Query execution coordinator.
//!
//! Ordering:
//! `parse header → compile query pack / injections / postprocess plan` (in `load.rs`) →
//! `parse host tree` (`capture.rs`) → `collect captures` (`capture.rs`) →
//! `dedupe + attach standalone docs + infer scopes` (`scope.rs`) →
//! `clean docs + finalize symbols` (`scope.rs`) →
//! `run postprocesses` (`postprocess.rs`) →
//! `run recursive injections` (`injections.rs`).

use anyhow::Result;
use tracing::instrument;

use codesight_core::StoredSymbol;

use super::ScmExtractor;
use super::capture::{collect_raw_captures, parse_host_tree};
use super::injections::run_injections;
use super::postprocess::run_postprocesses;
use super::scope::{attach_standalone_docs, dedupe_captures, infer_scopes_and_finalize_symbols};

#[instrument(
    level = "debug",
    skip(ext, source),
    fields(lang = ext.language_name(), path = file_path)
)]
pub(super) fn extract_symbols(
    ext: &ScmExtractor,
    file_path: &str,
    source: &str,
) -> Result<Vec<StoredSymbol>> {
    let tree = parse_host_tree(ext, source)?;

    let mut batch = collect_raw_captures(ext, source, &tree)?;
    dedupe_captures(&mut batch.raws);
    attach_standalone_docs(&mut batch.raws, &batch.standalone_docs);
    let mut symbols = infer_scopes_and_finalize_symbols(ext, file_path, source, &mut batch.raws);
    run_postprocesses(ext, file_path, source, &mut symbols);

    // Recursive injections: packs with an optional `injections.scm` may delegate
    // embedded language spans back through the registry. Delegated symbols are
    // already remapped to host coordinates by `containers::remap_symbols`.
    symbols.extend(run_injections(ext, file_path, source, &tree));

    symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
    Ok(symbols)
}
