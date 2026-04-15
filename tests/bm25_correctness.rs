//! Integration tests: bm25 correctness.

use codesight_retrieval::bm25_reference::{
    self, Bm25CorpusStats, Bm25SymbolRow, SymbolTokenCache, sym_token_bag, unique_dl,
};

#[test]
fn bm25_correctness_suite() {
    run_all();
}

fn run_all() {
    case_test_dl_is_unique_token_count();
    case_test_dl_not_inflated_by_field_reps();
    case_test_dl_matches_set_of_tokens();
    case_test_dl_no_inflation_compared_to_pre_t10();
    case_test_dl_consistent_on_second_call();
    case_test_avgdl_not_inflated();
    case_test_compute_bm25_rewrites_dl();
    case_test_bm25_corpus_size_equals_symbol_count();
    case_test_dl_consistent_after_compute();
    case_test_idf_formula_correct();
    case_test_retained_symbol_stale_dl_corrected();
}

fn case_test_dl_is_unique_token_count() {
    let bag = sym_token_bag("parse_file", "", "", "", &[]);
    assert_eq!(bag.len(), 6);
    assert_eq!(unique_dl(&bag), 2);
}

fn case_test_dl_not_inflated_by_field_reps() {
    let bag = sym_token_bag("foo", "foo(x)", "foo does something", "", &[]);
    let dl = unique_dl(&bag);
    assert!(bag.len() > dl);
    assert_eq!(
        dl,
        bag.iter().collect::<std::collections::HashSet<_>>().len()
    );
}

fn case_test_dl_matches_set_of_tokens() {
    let bag = sym_token_bag(
        "process_data",
        "process_data(df: DataFrame) -> dict",
        "process a data frame",
        "Process the input data and return results.",
        &[String::from("data"), String::from("process")],
    );
    let dl = unique_dl(&bag);
    assert_eq!(
        dl,
        bag.iter().collect::<std::collections::HashSet<_>>().len()
    );
}

fn case_test_dl_no_inflation_compared_to_pre_t10() {
    let bag = sym_token_bag("handler", "handler(req)", "", "", &[]);
    let dl = unique_dl(&bag);
    let uniq = bag.iter().collect::<std::collections::HashSet<_>>().len();
    assert!(uniq < bag.len());
    assert_eq!(dl, uniq);
    assert!(dl < bag.len());
}

fn case_test_dl_consistent_on_second_call() {
    let mut sym = SymbolTokenCache::new("connect", "connect(host, port)", "", "", &[]);
    let dl_first = sym.dl;
    sym.touch_again();
    assert_eq!(sym.dl, dl_first);
    assert_eq!(sym.dl, unique_dl(&sym.token_bag));
}

fn case_test_avgdl_not_inflated() {
    let mut syms = vec![
        Bm25SymbolRow::new("parse", "", "", "", &[]),
        Bm25SymbolRow::new("render", "", "", "", &[]),
        Bm25SymbolRow::new("connect", "", "", "", &[]),
    ];
    let Bm25CorpusStats { avgdl, .. } = bm25_reference::compute_bm25(&mut syms);
    assert!(
        (avgdl - 1.0).abs() < 1e-9,
        "Expected avgdl=1.0 (unique), got {avgdl}"
    );
}

fn case_test_compute_bm25_rewrites_dl() {
    let mut sym = Bm25SymbolRow::new("alpha", "", "", "", &[]);
    sym.dl = 999;
    bm25_reference::compute_bm25(std::slice::from_mut(&mut sym));
    assert_ne!(sym.dl, 999);
    assert_eq!(sym.dl, unique_dl(&sym.token_bag));
}

fn case_test_bm25_corpus_size_equals_symbol_count() {
    let mut syms: Vec<_> = (0..5)
        .map(|i| Bm25SymbolRow::new(&format!("func_{i}"), "", "", "", &[]))
        .collect();
    let Bm25CorpusStats { inverted, .. } = bm25_reference::compute_bm25(&mut syms);
    let mut all = std::collections::HashSet::new();
    for idxs in inverted.values() {
        for &i in idxs {
            all.insert(i);
        }
    }
    assert_eq!(all, [0, 1, 2, 3, 4].into_iter().collect());
}

fn case_test_dl_consistent_after_compute() {
    let mut syms = vec![
        Bm25SymbolRow::new("alpha", "", "first function", "", &[]),
        Bm25SymbolRow::new("beta", "", "second function here", "", &[]),
        Bm25SymbolRow::new("gamma", "", "", "", &[]),
    ];
    bm25_reference::compute_bm25(&mut syms);
    for sym in &syms {
        assert_eq!(
            sym.dl,
            unique_dl(&sym.token_bag),
            "Symbol {}: dl={} inconsistent",
            sym.name,
            sym.dl
        );
    }
}

fn case_test_idf_formula_correct() {
    let mut syms = vec![
        Bm25SymbolRow::new("shared_alpha", "", "", "", &[]),
        Bm25SymbolRow::new("shared_beta", "", "", "", &[]),
        Bm25SymbolRow::new("unique_func", "", "", "", &[]),
    ];
    let Bm25CorpusStats { idf, .. } = bm25_reference::compute_bm25(&mut syms);
    let n = 3.0_f64;
    let expected_shared: f64 = ((n - 2.0 + 0.5) / (2.0 + 0.5) + 1.0).ln();
    assert!((*idf.get("shared").unwrap() - expected_shared).abs() < 1e-9);
    let expected_unique: f64 = ((n - 1.0 + 0.5) / (1.0 + 0.5) + 1.0).ln();
    assert!((*idf.get("unique").unwrap() - expected_unique).abs() < 1e-9);
}

fn case_test_retained_symbol_stale_dl_corrected() {
    let mut sym = Bm25SymbolRow::new("process", "process(x, y)", "", "", &[]);
    let inflated_dl = sym.token_bag.len();
    sym.dl = inflated_dl;
    bm25_reference::compute_bm25(std::slice::from_mut(&mut sym));
    let canonical = unique_dl(&sym.token_bag);
    assert_eq!(
        sym.dl, canonical,
        "After rebuild, dl={} should be canonical {canonical}, not inflated {inflated_dl}",
        sym.dl
    );
}
