//! Golden test for `evaluate_impact analysis=path_rename` — the 15
//! canonical Rust import/path shapes we promise to rewrite (or explicitly
//! leave alone). These tests drive `preview_path_rename` directly so
//! they stay fast and don't need the full service.

use std::fs;

use codesight_analysis::impact::{preview_path_rename, summarize_path_rename};
use codesight_core::pipeline::IndexedFileUpdate;
use codesight_storage::storage::IndexStore;
use tempfile::TempDir;

fn run(source: &str, from: &str, to: &str) -> (usize, usize, String) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let file_abs = root.join("src/lib.rs");
    fs::create_dir_all(file_abs.parent().unwrap()).unwrap();
    fs::write(&file_abs, source).unwrap();

    let db = root.join(".idx/code.db");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let store = IndexStore::open(db).unwrap();
    let update = IndexedFileUpdate {
        file_path: "src/lib.rs".to_string(),
        language: codesight_core::LanguageId::intern("rust"),
        size_bytes: source.len() as u64,
        modified_ns: 0,
        content_hash: "0".to_string(),
        symbols: Vec::new(),
    };
    store
        .apply_code_index(
            "test",
            &root,
            "2026-04-23T00:00:00Z",
            1,
            &[],
            &[update],
            &[],
            &[],
        )
        .unwrap();

    let plan = preview_path_rename(&store, "rust", from, to, None, "warn")
        .unwrap()
        .expect("rust language should expose a path_rename engine");
    let (files, changes) = summarize_path_rename(&plan);
    let updated = plan
        .files_affected
        .iter()
        .find(|f| f.path == "src/lib.rs")
        .map(|f| f.updated_source.clone())
        .unwrap_or_default();
    (files, changes, updated)
}

#[test]
fn shape_01_plain_use() {
    let src = "use crate::a::Foo;\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("use crate::a::Bar;"));
}

#[test]
fn shape_02_pub_use() {
    let src = "pub use crate::a::Foo;\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("pub use crate::a::Bar;"));
}

#[test]
fn shape_03_use_as_alias() {
    let src = "use crate::a::Foo as F;\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("use crate::a::Bar as F;"));
}

#[test]
fn shape_04_fully_qualified_call() {
    let src = "fn run() { crate::a::Foo::do_it(); }\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("crate::a::Bar::do_it"));
}

#[test]
fn shape_05_turbofish() {
    let src = "fn run() { let _: Vec<crate::a::Foo> = Vec::<crate::a::Foo>::new(); }\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 2));
    assert!(!updated.contains("crate::a::Foo"));
}

#[test]
fn shape_06_trait_impl_bound() {
    let src = "impl crate::a::Foo for MyType {}\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("impl crate::a::Bar for MyType"));
}

#[test]
fn shape_07_grouped_use_single_item() {
    let src = "use crate::a::{Foo};\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("use crate::a::{Bar};"));
}

#[test]
fn shape_08_grouped_use_multi_item_middle() {
    let src = "use crate::a::{One, Foo, Two};\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("use crate::a::{One, Bar, Two};"));
}

#[test]
fn shape_09_grouped_use_with_alias() {
    let src = "use crate::a::{Foo as F, Two};\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("use crate::a::{Bar as F, Two};"));
}

#[test]
fn shape_10_grouped_use_pub() {
    let src = "pub use crate::a::{Foo};\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("pub use crate::a::{Bar};"));
}

#[test]
fn shape_11_comment_noise() {
    let src = "// crate::a::Foo in a comment should not match\nuse crate::a::Foo;\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (1, 1));
    assert!(updated.contains("// crate::a::Foo in a comment"));
    assert!(updated.contains("use crate::a::Bar;"));
}

#[test]
fn shape_12_partial_prefix_not_matched() {
    let src = "use crate::a::FooBar;\n";
    let (files, changes, _) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (0, 0));
}

#[test]
fn shape_13_longer_prefix_not_matched() {
    let src = "use crate::aa::Foo;\n";
    let (files, changes, _) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (0, 0));
}

#[test]
fn shape_14_string_literal_not_matched() {
    let src = "const S: &str = \"crate::a::Foo\";\n";
    let (files, changes, _) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!((files, changes), (0, 0));
}

#[test]
fn shape_15_multiple_sites_one_file() {
    let src = "\
use crate::a::Foo;\n\
fn make() -> crate::a::Foo { crate::a::Foo }\n";
    let (files, changes, updated) = run(src, "crate::a::Foo", "crate::a::Bar");
    assert_eq!(files, 1);
    assert!(changes >= 3);
    assert!(!updated.contains("crate::a::Foo"));
}
