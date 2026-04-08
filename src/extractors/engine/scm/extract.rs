//! Query execution coordinator.
//!
//! Ordering:
//! `parse header → compile query pack / injections / postprocess plan` (in `load.rs`) →
//! `collect captures` (`capture.rs`) → `dedupe + attach standalone docs + infer scopes`
//! (`scope.rs`) → `clean docs + finalize symbols` (`scope.rs`) →
//! `run postprocesses` (`postprocess.rs`).

use anyhow::Result;

use crate::model::StoredSymbol;

use super::ScmExtractor;
use super::capture::collect_raw_captures;
use super::postprocess::run_postprocesses;
use super::scope::{attach_standalone_docs, dedupe_captures, infer_scopes_and_finalize_symbols};

pub(super) fn extract_symbols(
    ext: &ScmExtractor,
    file_path: &str,
    source: &str,
) -> Result<Vec<StoredSymbol>> {
    // Pack loading precompiles optional injections.scm so future recursive container
    // execution plugs into the same coordinator without changing this facade again.
    let _recursive_injection_plan = ext.compiled.injections_query.as_ref();

    let mut batch = collect_raw_captures(ext, source)?;
    dedupe_captures(&mut batch.raws);
    attach_standalone_docs(&mut batch.raws, &batch.standalone_docs);
    let mut symbols = infer_scopes_and_finalize_symbols(ext, file_path, source, &mut batch.raws);
    run_postprocesses(ext, file_path, source, &mut symbols);

    symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
    Ok(symbols)
}
