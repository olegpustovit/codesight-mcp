//! Dense-embedding front-end.
//!
//! Wraps `model2vec-rs` with a deterministic chunk-text policy (see
//! §7 of `IDF_REPLACEMENT.md`) and a batching encode API. Loading is
//! fallible and cached: one `OnceLock` per `CodeService`, surfacing
//! `SEMANTIC_FALLBACK` when the model can't be reached.
//!
//! Model resolution order, all optional:
//! 1. `CODESIGHT_EMBEDDING_MODEL_PATH` — local folder with
//!    `tokenizer.json` + `model.safetensors` + `config.json`.
//! 2. `CODESIGHT_EMBEDDING_MODEL_REPO` — HuggingFace repo id.
//! 3. Fall back to `minishlab/potion-base-8M` (the canonical 256-d
//!    model pinned in `codesight_storage::EMBEDDING_DIM`).

use std::sync::OnceLock;

use codesight_storage::EMBEDDING_DIM;
use model2vec_rs::model::StaticModel;

const DEFAULT_REPO: &str = "minishlab/potion-base-8M";

/// Opaque embedder handle. `Arc<Embedder>` is the shared form held on
/// `CodeService`; callers go through [`encode`] / [`encode_query`].
pub struct Embedder {
    inner: StaticModel,
}

impl Embedder {
    /// Load the embedder, consulting env then defaults. Pure function —
    /// no disk writes. The caller is expected to cache success into a
    /// `OnceLock` so repeated retrievals don't re-hit hf-hub.
    pub fn load() -> anyhow::Result<Self> {
        let (handle, where_from): (String, &'static str) =
            if let Ok(p) = std::env::var("CODESIGHT_EMBEDDING_MODEL_PATH") {
                (p, "path")
            } else if let Ok(r) = std::env::var("CODESIGHT_EMBEDDING_MODEL_REPO") {
                (r, "repo")
            } else {
                (DEFAULT_REPO.to_string(), "default")
            };
        tracing::debug!(where_from, handle, "loading model2vec embedder");
        let model = StaticModel::from_pretrained(&handle, None, None, None)
            .map_err(|e| anyhow::anyhow!("model2vec load failed ({where_from}={handle}): {e}"))?;
        Ok(Self { inner: model })
    }

    /// Embed a batch of chunk texts. Empty input → empty output.
    /// The model always emits `EMBEDDING_DIM`-length vectors; this is
    /// checked once on the first row.
    pub fn encode(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let out = self.inner.encode(texts);
        if let Some(first) = out.first()
            && first.len() != EMBEDDING_DIM
        {
            anyhow::bail!(
                "embedder produced dim {}, expected {EMBEDDING_DIM} — wrong model?",
                first.len()
            );
        }
        Ok(out)
    }

    /// Encode a single query string.
    pub fn encode_query(&self, query: &str) -> anyhow::Result<Vec<f32>> {
        let one = self.inner.encode_single(query);
        if one.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "embedder produced dim {}, expected {EMBEDDING_DIM}",
                one.len()
            );
        }
        Ok(one)
    }
}

/// Assemble the text that represents a code symbol for embedding.
/// Shape matches `IDF_REPLACEMENT.md §7.1`.
pub fn symbol_chunk_text(
    kind: &str,
    name: &str,
    signature: &str,
    summary: &str,
    source: &str,
) -> String {
    // Source-prefix limit: match the plan's "functions/methods: 512 chars,
    // types: 256" rule. Here we apply the more generous cap and trust the
    // source itself to be short for simple symbols.
    let cap = match kind {
        "function" | "method" => 512,
        _ => 256,
    };
    let src = take_chars(source, cap);
    let mut out = String::with_capacity(name.len() + signature.len() + summary.len() + src.len());
    out.push_str(kind);
    out.push(' ');
    out.push_str(name);
    out.push('\n');
    out.push_str(signature);
    out.push('\n');
    out.push_str(summary);
    out.push('\n');
    out.push_str(&src);
    out
}

/// Assemble the text that represents a doc section for embedding.
/// Shape matches `IDF_REPLACEMENT.md §7.2`.
pub fn section_chunk_text(
    heading: &str,
    heading_path: &str,
    summary: &str,
    content: &str,
) -> String {
    let body = take_chars(content, 512);
    format!("{heading}\n{heading_path}\n{summary}\n{body}")
}

fn take_chars(text: &str, cap: usize) -> String {
    // char-safe truncation: bytewise slicing would split multibyte chars.
    text.chars().take(cap).collect()
}

/// One-shot embedder slot, lazy-loaded on first use. Thread-safe.
///
/// `Ok(Arc<Embedder>)` — model loaded, ready to use.
/// `Err(String)` — recorded failure reason; subsequent calls short-circuit
/// to the same reason (no retry storms).
pub struct EmbedderSlot(OnceLock<Result<std::sync::Arc<Embedder>, String>>);

impl EmbedderSlot {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }

    /// Load-or-return-cached. The closure is only invoked on first call.
    pub fn get(&self) -> Result<&std::sync::Arc<Embedder>, &str> {
        let slot = self.0.get_or_init(|| {
            Embedder::load()
                .map(std::sync::Arc::new)
                .map_err(|e| e.to_string())
        });
        slot.as_ref().map_err(String::as_str)
    }
}

impl Default for EmbedderSlot {
    fn default() -> Self {
        Self::new()
    }
}
