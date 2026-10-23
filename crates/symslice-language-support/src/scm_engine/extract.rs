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

use symslice_core::StoredSymbol;

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
    let diagnostics = std::env::var("SYMSLICE_INGEST_DIAGNOSTICS").as_deref() == Ok("1");
    let diagnostic = |phase: &str, count: usize| {
        if !diagnostics {
            return;
        }
        let rss_bytes = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix("VmRSS:"))?
                    .split_whitespace()
                    .next()?
                    .parse::<usize>()
                    .ok()?
                    .checked_mul(1024)
            });
        let grammar = &ext.compiled().grammar;
        tracing::info!(
            phase,
            path = file_path,
            language = ext.language_name(),
            source_bytes = source.len(),
            count,
            grammar_abi = grammar.abi_version(),
            grammar_node_kinds = grammar.node_kind_count(),
            grammar_fields = grammar.field_count(),
            grammar_states = grammar.parse_state_count(),
            rss_bytes,
            "SCM ingest diagnostic"
        );
    };
    diagnostic("scm_parse_start", 0);
    let tree = parse_host_tree(ext, source)?;
    diagnostic("scm_parse_end", 0);

    diagnostic("scm_capture_start", 0);
    let mut batch = collect_raw_captures(ext, source, &tree)?;
    diagnostic("scm_capture_end", batch.raws.len());
    diagnostic("scm_dedupe_start", batch.raws.len());
    dedupe_captures(&mut batch.raws);
    diagnostic("scm_dedupe_end", batch.raws.len());
    diagnostic("scm_docs_start", batch.raws.len());
    attach_standalone_docs(&mut batch.raws, &batch.standalone_docs);
    diagnostic("scm_docs_end", batch.raws.len());
    diagnostic("scm_finalize_start", batch.raws.len());
    let mut symbols = infer_scopes_and_finalize_symbols(ext, file_path, source, &mut batch.raws);
    diagnostic("scm_finalize_end", symbols.len());
    diagnostic("scm_postprocess_start", symbols.len());
    run_postprocesses(ext, file_path, source, &mut symbols);
    diagnostic("scm_postprocess_end", symbols.len());

    // Recursive injections: packs with an optional `injections.scm` may delegate
    // embedded language spans back through the registry. Delegated symbols are
    // already remapped to host coordinates by `containers::remap_symbols`.
    diagnostic("scm_injections_start", symbols.len());
    symbols.extend(run_injections(ext, file_path, source, &tree));
    diagnostic("scm_injections_end", symbols.len());

    symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
    Ok(symbols)
}
