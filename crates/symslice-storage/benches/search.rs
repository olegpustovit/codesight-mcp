//! Storage search microbench.
//!
//! Establishes a baseline for `IndexStore::search_symbols` (FTS5 + reranking)
//! against a synthetic in-memory fixture. Run with:
//!
//! ```text
//! cargo bench -p symslice-storage --bench search
//! ```
//!
//! Nix-on-macOS gotcha: linking the release-mode bench through `libsqlite3-sys`
//! requires `libiconv`, which the Nix shell does not surface to the system
//! linker. Pass the path explicitly when running here, e.g.
//!
//! ```text
//! RUSTFLAGS="-L $(nix eval --raw nixpkgs#libiconv)/lib" \
//!     cargo bench -p symslice-storage --bench search
//! ```

use std::path::Path;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use symslice_storage::core::pipeline::{FileFingerprint, IndexedFileUpdate, StoredSymbol};
use symslice_storage::core::symbols::{ExtractionTier, SymbolKind};
use symslice_storage::storage::IndexStore;
use tempfile::TempDir;

const NUM_FILES: usize = 50;
const SYMBOLS_PER_FILE: usize = 20;

fn build_fixture() -> (TempDir, IndexStore) {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("index.sqlite3");
    let store = IndexStore::open(db_path).expect("open");

    let mut files = Vec::with_capacity(NUM_FILES);
    let mut updates = Vec::with_capacity(NUM_FILES);

    for f in 0..NUM_FILES {
        let file_path = format!("src/module_{f}/mod.rs");
        files.push(FileFingerprint {
            file_path: file_path.clone(),
            language: symslice_core::RUST,
            size_bytes: 4096,
            modified_ns: 0,
            content_hash: format!("hash-{f}"),
        });

        let mut symbols = Vec::with_capacity(SYMBOLS_PER_FILE);
        for s in 0..SYMBOLS_PER_FILE {
            let name = format!("process_request_{f}_{s}");
            symbols.push(StoredSymbol {
                id: format!("{file_path}:{name}"),
                extraction_tier: ExtractionTier::CuratedQueryPack,
                language: symslice_core::RUST,
                file_path: file_path.clone(),
                qualified_name: format!("module_{f}::{name}"),
                name,
                kind: SymbolKind::Function,
                signature: format!("fn process_request_{f}_{s}(req: Request) -> Response"),
                summary: "handles an HTTP request and returns a structured response".to_string(),
                start_line: (s * 10) as u32 + 1,
                end_line: (s * 10) as u32 + 9,
                start_byte: s * 200,
                end_byte: s * 200 + 180,
                source_hash: format!("sym-{f}-{s}"),
                source_text: format!("fn process_request_{f}_{s}() {{ /* body */ }}"),
                file_size_bytes: 4096,
            });
        }

        updates.push(IndexedFileUpdate {
            file_path,
            language: symslice_core::RUST,
            size_bytes: 4096,
            modified_ns: 0,
            content_hash: format!("hash-{f}"),
            symbols,
        });
    }

    store
        .apply_code_index(
            "bench-repo",
            Path::new("/tmp/bench-repo"),
            "2026-04-10T00:00:00Z",
            1,
            &files,
            &updates,
            &[],
            &[],
        )
        .expect("apply_code_index");

    (dir, store)
}

fn bench_search(c: &mut Criterion) {
    let (_dir, store) = build_fixture();

    let mut group = c.benchmark_group("storage_search");
    group.bench_function("symbol_fts_token", |b| {
        b.iter(|| {
            let hits = store
                .search_symbols(black_box("process"), None, None, &[], 25)
                .expect("search");
            black_box(hits);
        });
    });
    group.bench_function("symbol_fts_phrase", |b| {
        b.iter(|| {
            let hits = store
                .search_symbols(black_box("process_request"), None, None, &[], 25)
                .expect("search");
            black_box(hits);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
