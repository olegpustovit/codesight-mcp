//! SCM extractor microbench.
//!
//! Establishes a baseline for `LanguageExtractor::extract` on the curated query
//! packs for rust, typescript, and python. Run with:
//!
//! ```text
//! cargo bench -p codesight-extractors --bench scm
//! ```

use std::path::Path;

use codesight_extractors::extractors::ExtractorRegistry;
use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};

const RUST_SOURCE: &str = include_str!("fixtures/sample.rs");
const TYPESCRIPT_SOURCE: &str = include_str!("fixtures/sample.ts");
const PYTHON_SOURCE: &str = include_str!("fixtures/sample.py");

fn bench_scm(c: &mut Criterion) {
    let registry = ExtractorRegistry::default();

    let mut group = c.benchmark_group("scm_extract");
    for (label, path, source) in [
        ("rust", "sample.rs", RUST_SOURCE),
        ("typescript", "sample.ts", TYPESCRIPT_SOURCE),
        ("python", "sample.py", PYTHON_SOURCE),
    ] {
        let extractor = registry
            .extractor_for_path(Path::new(path))
            .unwrap_or_else(|| panic!("no extractor for {label}"));
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_function(label, |b| {
            b.iter(|| {
                let symbols = extractor
                    .extract(black_box(path), black_box(source))
                    .expect("extract");
                black_box(symbols);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_scm);
criterion_main!(benches);
