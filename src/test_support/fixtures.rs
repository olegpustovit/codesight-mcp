//! Paths to checked-in trees under `tests/fixtures/`.

/// Root of the checked-in documentation fixture tree under `tests/fixtures/doc_reference/`.
pub fn doc_reference_fixtures_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests/fixtures/doc_reference")
}

/// Root of the checked-in code fixture tree under `tests/fixtures/code_reference/`.
pub fn code_reference_fixtures_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests/fixtures/code_reference")
}
