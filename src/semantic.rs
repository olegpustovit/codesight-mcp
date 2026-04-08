//! Semantic search: TF-IDF bag-of-words embeddings, cosine similarity,
//! embedding store, and provider detection.
//!
//! This is a self-contained, no-external-API-needed embedding system.
//! It tokenizes symbol names + source text, computes TF-IDF vectors,
//! and stores them for cosine-similarity retrieval.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of `detect_embedding_provider()`: provider name + model identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProvider {
    pub provider: String,
    pub model: String,
}

/// Detect which embedding provider is configured from environment variables.
///
/// Priority order (matching reference behavior):
/// 1. `CODESIGHT_EMBED_MODEL` → "sentence_transformers"
/// 2. `GOOGLE_API_KEY` + `GOOGLE_EMBED_MODEL` → "gemini"
/// 3. `OPENAI_API_KEY` + `OPENAI_EMBED_MODEL` → "openai"
///
/// Returns `None` when no provider is fully configured.
pub fn detect_embedding_provider() -> Option<EmbeddingProvider> {
    if let Ok(model) = std::env::var("CODESIGHT_EMBED_MODEL")
        && !model.is_empty()
    {
        return Some(EmbeddingProvider {
            provider: "sentence_transformers".to_string(),
            model,
        });
    }
    if let (Ok(key), Ok(model)) = (
        std::env::var("GOOGLE_API_KEY"),
        std::env::var("GOOGLE_EMBED_MODEL"),
    ) && !key.is_empty()
        && !model.is_empty()
    {
        return Some(EmbeddingProvider {
            provider: "gemini".to_string(),
            model,
        });
    }
    if let (Ok(key), Ok(model)) = (
        std::env::var("OPENAI_API_KEY"),
        std::env::var("OPENAI_EMBED_MODEL"),
    ) && !key.is_empty()
        && !model.is_empty()
    {
        return Some(EmbeddingProvider {
            provider: "openai".to_string(),
            model,
        });
    }
    None
}

/// Cosine similarity between two vectors.
/// Returns 0.0 for zero vectors or mismatched lengths.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// In-memory + file-persistent embedding store.
///
/// Stores embeddings as `symbol_id -> Vec<f64>`. Optionally persists to a
/// JSON file so embeddings survive across process restarts.
#[derive(Debug)]
pub struct EmbeddingStore {
    embeddings: HashMap<String, Vec<f64>>,
    dimension: Option<usize>,
    model: Option<String>,
    path: Option<PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredData {
    dimension: Option<usize>,
    model: Option<String>,
    embeddings: HashMap<String, Vec<f64>>,
}

impl EmbeddingStore {
    /// Create a new in-memory store (no persistence).
    pub fn new() -> Self {
        Self {
            embeddings: HashMap::new(),
            dimension: None,
            model: None,
            path: None,
        }
    }

    /// Create a store backed by a file. If the file exists, loads from it.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut store = Self {
            embeddings: HashMap::new(),
            dimension: None,
            model: None,
            path: Some(path.clone()),
        };
        if path.exists()
            && let Ok(data) = fs::read_to_string(&path)
            && let Ok(stored) = serde_json::from_str::<StoredData>(&data)
        {
            store.embeddings = stored.embeddings;
            store.dimension = stored.dimension;
            store.model = stored.model;
        }
        store
    }

    /// Set the expected dimension and model name.
    pub fn set_dimension(&mut self, dim: usize, model: &str) {
        self.dimension = Some(dim);
        self.model = Some(model.to_string());
        self.persist();
    }

    /// Get the current dimension.
    pub fn get_dimension(&self) -> Option<usize> {
        self.dimension
    }

    /// Store multiple embeddings at once.
    pub fn set_many(&mut self, items: HashMap<String, Vec<f64>>) {
        for (k, v) in items {
            self.embeddings.insert(k, v);
        }
        self.persist();
    }

    /// Retrieve the embedding for a symbol.
    pub fn get(&self, key: &str) -> Option<&Vec<f64>> {
        self.embeddings.get(key)
    }

    /// Delete specified embeddings.
    pub fn delete_many(&mut self, keys: &[&str]) {
        for k in keys {
            self.embeddings.remove(*k);
        }
        self.persist();
    }

    /// Number of stored embeddings.
    pub fn count(&self) -> usize {
        self.embeddings.len()
    }

    /// Check whether a symbol has an embedding.
    pub fn contains(&self, key: &str) -> bool {
        self.embeddings.contains_key(key)
    }

    /// Find the top-N most similar symbols to a query vector.
    pub fn search(&self, query: &[f64], top_n: usize) -> Vec<(String, f64)> {
        let mut scored: Vec<(String, f64)> = self
            .embeddings
            .iter()
            .map(|(k, v)| (k.clone(), cosine_similarity(query, v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_n);
        scored
    }

    /// All stored symbol IDs.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.embeddings.keys()
    }

    fn persist(&self) {
        if let Some(ref path) = self.path {
            let data = StoredData {
                dimension: self.dimension,
                model: self.model.clone(),
                embeddings: self.embeddings.clone(),
            };
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = serde_json::to_string(&data)
                .ok()
                .and_then(|json| fs::write(path, json).ok());
        }
    }
}

impl Default for EmbeddingStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── TF-IDF Embedding Engine ─────────────────────────────────────────────────

/// Tokenize text into lowercase alphanumeric tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Compute TF-IDF embeddings for a collection of documents.
///
/// Returns `(vocabulary, embeddings)` where each embedding is a Vec<f64>
/// with one entry per vocabulary term, in sorted vocabulary order.
pub fn compute_tfidf_embeddings(
    documents: &[(String, String)], // (id, text)
) -> (Vec<String>, HashMap<String, Vec<f64>>) {
    if documents.is_empty() {
        return (Vec::new(), HashMap::new());
    }

    // Tokenize all documents
    let doc_tokens: Vec<Vec<String>> = documents.iter().map(|(_, text)| tokenize(text)).collect();

    // Build vocabulary + document frequencies
    let mut df: HashMap<String, usize> = HashMap::new();
    for tokens in &doc_tokens {
        let unique: std::collections::HashSet<&String> = tokens.iter().collect();
        for tok in unique {
            *df.entry(tok.clone()).or_insert(0) += 1;
        }
    }

    // Sort vocabulary for deterministic ordering
    let mut vocab: Vec<String> = df.keys().cloned().collect();
    vocab.sort();
    let vocab_index: HashMap<&str, usize> = vocab
        .iter()
        .enumerate()
        .map(|(i, w)| (w.as_str(), i))
        .collect();

    let n_docs = documents.len() as f64;

    // Compute TF-IDF vectors
    let mut embeddings = HashMap::new();
    for (i, (id, _)) in documents.iter().enumerate() {
        let tokens = &doc_tokens[i];
        let mut tf: HashMap<&str, f64> = HashMap::new();
        for tok in tokens {
            *tf.entry(tok.as_str()).or_insert(0.0) += 1.0;
        }
        let n_terms = tokens.len() as f64;

        let mut vec = vec![0.0_f64; vocab.len()];
        for (term, count) in &tf {
            if let Some(&idx) = vocab_index.get(term) {
                let tf_val = count / n_terms.max(1.0);
                let idf_val = (n_docs / (*df.get(*term).unwrap_or(&1)) as f64).ln() + 1.0;
                vec[idx] = tf_val * idf_val;
            }
        }

        // L2 normalize
        let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }

        embeddings.insert(id.clone(), vec);
    }

    (vocab, embeddings)
}

/// Compute a TF-IDF query vector against an existing vocabulary + IDF stats.
pub fn compute_query_vector(
    query: &str,
    vocab: &[String],
    doc_count: usize,
    df: &HashMap<String, usize>,
) -> Vec<f64> {
    let tokens = tokenize(query);
    let vocab_index: HashMap<&str, usize> = vocab
        .iter()
        .enumerate()
        .map(|(i, w)| (w.as_str(), i))
        .collect();
    let n_docs = doc_count as f64;

    let mut tf: HashMap<&str, f64> = HashMap::new();
    for tok in &tokens {
        *tf.entry(tok.as_str()).or_insert(0.0) += 1.0;
    }
    let n_terms = tokens.len() as f64;

    let mut vec = vec![0.0_f64; vocab.len()];
    for (term, count) in &tf {
        if let Some(&idx) = vocab_index.get(term) {
            let tf_val = count / n_terms.max(1.0);
            let idf_val = (n_docs / (*df.get(*term).unwrap_or(&1)) as f64).ln() + 1.0;
            vec[idx] = tf_val * idf_val;
        }
    }

    // L2 normalize
    let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }

    vec
}

/// Complete TF-IDF model stored alongside the embedding store.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TfidfModel {
    pub vocab: Vec<String>,
    pub doc_count: usize,
    pub df: HashMap<String, usize>,
}

impl TfidfModel {
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_orthogonal() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
    }

    #[test]
    fn cosine_zero() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn cosine_mismatch() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn tfidf_basic() {
        let docs = vec![
            ("a".to_string(), "hello world".to_string()),
            ("b".to_string(), "hello rust".to_string()),
        ];
        let (vocab, emb) = compute_tfidf_embeddings(&docs);
        assert!(!vocab.is_empty());
        assert_eq!(emb.len(), 2);
        // "hello" appears in both docs -> lower IDF; "world"/"rust" unique -> higher
        let a = &emb["a"];
        let b = &emb["b"];
        let sim = cosine_similarity(a, b);
        // Should be between 0 and 1 (partial overlap)
        assert!(sim > 0.0 && sim < 1.0, "sim={sim}");
    }
}
