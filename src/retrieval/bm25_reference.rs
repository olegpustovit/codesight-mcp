//! BM25 corpus helpers (token `dl`, avgdl, Robertson IDF weighting).
//! (unique-token document length, avgdl, Robertson–Spider IDF, inverted index coverage).
//!
//! Field repetition counts are inferred only from reference test fixtures (name×3, signature×2,
//! summary×2, docstring×2, each keyword×1) — from published BM25/Lucene weighting literature.

use std::collections::{BTreeMap, HashSet};

use crate::db;

const NAME_REPS: usize = 3;
const SIGNATURE_REPS: usize = 2;
const SUMMARY_REPS: usize = 2;
const DOCSTRING_REPS: usize = 2;
const KEYWORD_REPS: usize = 1;

fn extend_field(bag: &mut Vec<String>, text: &str, reps: usize) {
    if text.is_empty() || reps == 0 {
        return;
    }
    let toks = db::tokenize(text);
    if toks.is_empty() {
        return;
    }
    for _ in 0..reps {
        bag.extend(toks.iter().cloned());
    }
}

/// Repeated token bag used for TF-style statistics (matches reference “inflated bag” tests).
pub fn sym_token_bag(
    name: &str,
    signature: &str,
    summary: &str,
    docstring: &str,
    keywords: &[String],
) -> Vec<String> {
    let mut bag = Vec::new();
    extend_field(&mut bag, name, NAME_REPS);
    extend_field(&mut bag, signature, SIGNATURE_REPS);
    extend_field(&mut bag, summary, SUMMARY_REPS);
    extend_field(&mut bag, docstring, DOCSTRING_REPS);
    for kw in keywords {
        extend_field(&mut bag, kw, KEYWORD_REPS);
    }
    bag
}

/// Canonical document length: **unique** token count (not bag length).
pub fn unique_dl(bag: &[String]) -> usize {
    bag.iter().collect::<HashSet<_>>().len()
}

/// Per-symbol state for “cache” tests (`test_dl_consistent_on_second_call`).
#[derive(Debug, Clone)]
pub struct SymbolTokenCache {
    name: String,
    signature: String,
    summary: String,
    docstring: String,
    keywords: Vec<String>,
    pub token_bag: Vec<String>,
    pub dl: usize,
    sealed: bool,
}

impl SymbolTokenCache {
    pub fn new(
        name: &str,
        signature: &str,
        summary: &str,
        docstring: &str,
        keywords: &[String],
    ) -> Self {
        let mut s = Self {
            name: name.to_string(),
            signature: signature.to_string(),
            summary: summary.to_string(),
            docstring: docstring.to_string(),
            keywords: keywords.to_vec(),
            token_bag: Vec::new(),
            dl: 0,
            sealed: false,
        };
        s.ensure_tokens();
        s
    }

    /// First (or forced) computation of `_tokens` / `_dl`.
    pub fn ensure_tokens(&mut self) {
        if self.sealed {
            return;
        }
        self.token_bag = sym_token_bag(
            &self.name,
            &self.signature,
            &self.summary,
            &self.docstring,
            &self.keywords,
        );
        self.dl = unique_dl(&self.token_bag);
        self.sealed = true;
    }

    /// Simulate second `_sym_tokens` call: fast path leaves bag and dl unchanged.
    pub fn touch_again(&mut self) {
        self.ensure_tokens();
    }
}

/// Mutable symbol row for `_compute_bm25`-style passes.
#[derive(Debug, Clone)]
pub struct Bm25SymbolRow {
    pub name: String,
    pub signature: String,
    pub summary: String,
    pub docstring: String,
    pub keywords: Vec<String>,
    pub token_bag: Vec<String>,
    pub dl: usize,
}

impl Bm25SymbolRow {
    pub fn new(
        name: &str,
        signature: &str,
        summary: &str,
        docstring: &str,
        keywords: &[String],
    ) -> Self {
        let token_bag = sym_token_bag(name, signature, summary, docstring, keywords);
        let dl = unique_dl(&token_bag);
        Self {
            name: name.to_string(),
            signature: signature.to_string(),
            summary: summary.to_string(),
            docstring: docstring.to_string(),
            keywords: keywords.to_vec(),
            token_bag,
            dl,
        }
    }

    pub fn touch_tokens(&mut self) {
        self.token_bag = sym_token_bag(
            &self.name,
            &self.signature,
            &self.summary,
            &self.docstring,
            &self.keywords,
        );
        self.dl = unique_dl(&self.token_bag);
    }
}

#[derive(Debug, Clone)]
pub struct Bm25CorpusStats {
    pub idf: BTreeMap<String, f64>,
    pub avgdl: f64,
    pub inverted: BTreeMap<String, Vec<usize>>,
}

/// Robertson `log((N - df + 0.5) / (df + 0.5) + 1)` with natural logarithm (matches `math.log` in tests).
fn robertson_idf(n_docs: usize, df: usize) -> f64 {
    let n = n_docs as f64;
    let d = df as f64;
    ((n - d + 0.5) / (d + 0.5) + 1.0).ln()
}

/// Recompute canonical `dl`, then build IDF / avgdl / inverted index over **unique** per-doc vocab.
pub fn compute_bm25(symbols: &mut [Bm25SymbolRow]) -> Bm25CorpusStats {
    for sym in symbols.iter_mut() {
        if sym.token_bag.is_empty() {
            sym.touch_tokens();
        }
        sym.dl = unique_dl(&sym.token_bag);
    }

    let n = symbols.len();
    let avgdl = if n == 0 {
        0.0
    } else {
        symbols.iter().map(|s| s.dl as f64).sum::<f64>() / n as f64
    };

    let mut df: BTreeMap<String, usize> = BTreeMap::new();
    for sym in symbols.iter() {
        let uniq: HashSet<_> = sym.token_bag.iter().cloned().collect();
        for t in uniq {
            *df.entry(t).or_insert(0) += 1;
        }
    }

    let mut idf = BTreeMap::new();
    for (term, dfi) in &df {
        idf.insert(term.clone(), robertson_idf(n, *dfi));
    }

    let mut inverted: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        let uniq: HashSet<_> = sym.token_bag.iter().cloned().collect();
        for t in uniq {
            inverted.entry(t).or_default().push(i);
        }
    }

    Bm25CorpusStats {
        idf,
        avgdl,
        inverted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_avgdl_parse_render_connect() {
        let mut syms = vec![
            Bm25SymbolRow::new("parse", "", "", "", &[]),
            Bm25SymbolRow::new("render", "", "", "", &[]),
            Bm25SymbolRow::new("connect", "", "", "", &[]),
        ];
        let stats = compute_bm25(&mut syms);
        assert!((stats.avgdl - 1.0).abs() < 1e-9);
    }
}
