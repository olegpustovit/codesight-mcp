//! Fuzzy symbol-name helpers for catalog search (trigram Jaccard + Levenshtein).
//! Used by `search_catalog` for trigram Jaccard + Levenshtein augmentation.

use std::collections::HashSet;

/// Reference `_trigrams`: lowercase, length&lt;3 returns the whole string as one token set member.
pub fn trigrams(s: &str) -> HashSet<String> {
    let s = s.to_ascii_lowercase();
    if s.is_empty() {
        return HashSet::new();
    }
    if s.len() < 3 {
        return HashSet::from([s]);
    }
    let mut out = HashSet::new();
    for w in s.as_bytes().windows(3) {
        out.insert(String::from_utf8_lossy(w).into_owned());
    }
    out
}

pub fn trigram_jaccard(a: &str, b: &str) -> f64 {
    let ta = trigrams(a);
    let tb = trigrams(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    }
}

/// Reference `_edit_distance` (Levenshtein on Unicode scalar values).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (cur[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

pub fn normalize_fuzzy_compare(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

#[derive(Debug, Clone)]
pub struct FuzzyPairScore {
    pub jaccard: f64,
    pub edit_distance: u32,
    /// Combined score in (0,1] suitable for `fuzzy_similarity`.
    pub similarity: f64,
}

pub fn score_name_pair(query: &str, symbol_name: &str) -> FuzzyPairScore {
    let q = normalize_fuzzy_compare(query);
    let n = normalize_fuzzy_compare(symbol_name);
    let jaccard = trigram_jaccard(&q, &n);
    let ed = edit_distance(&q, &n) as u32;
    let denom = q.len().max(n.len()).max(1);
    let edit_sim = 1.0f64 - (f64::from(ed) / denom as f64);
    let similarity = jaccard.max(edit_sim).clamp(0.0, 1.0);
    FuzzyPairScore {
        jaccard,
        edit_distance: ed,
        similarity,
    }
}

pub fn fuzzy_candidate_passes(
    score: &FuzzyPairScore,
    threshold: f64,
    max_edit_distance: Option<u32>,
) -> bool {
    if score.jaccard >= threshold {
        return true;
    }
    if let Some(limit) = max_edit_distance {
        if score.edit_distance <= limit {
            return true;
        }
    }
    false
}

/// Tokenize a symbol name for fuzzy matching (underscore + camel segments).
pub fn tokenize_name(name: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for segment in name.split('_') {
        if segment.is_empty() {
            continue;
        }
        let parts = split_camel(segment);
        for mut p in parts {
            p.make_ascii_lowercase();
            if p.len() >= 2 {
                out.insert(p);
            }
        }
    }
    out
}

fn flush(buf: &mut String, out: &mut Vec<String>) {
    if !buf.is_empty() {
        out.push(std::mem::take(buf));
    }
}

fn split_camel(part: &str) -> Vec<String> {
    let mut raw_parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in part.chars() {
        if c.is_uppercase()
            && !cur.is_empty()
            && cur.chars().last().is_some_and(|lc| lc.is_lowercase())
        {
            flush(&mut cur, &mut raw_parts);
        }
        cur.push(c);
    }
    flush(&mut cur, &mut raw_parts);
    if raw_parts.is_empty() && !part.is_empty() {
        raw_parts.push(part.to_string());
    }
    raw_parts
}

pub fn relatedness_score(
    source_name: &str,
    source_file: &str,
    other_name: &str,
    other_file: &str,
) -> f64 {
    let same_file = source_file == other_file;
    let a = tokenize_name(source_name);
    let b = tokenize_name(other_name);
    if a.is_empty() && b.is_empty() {
        return if same_file { 0.5 } else { 0.0 };
    }
    let inter = a.intersection(&b).count() as f64;
    let union = a.union(&b).count().max(1) as f64;
    let j = inter / union;
    if same_file { 0.5 + j * 0.75 } else { j }
}
