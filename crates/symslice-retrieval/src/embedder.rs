//! Dense-embedding front-end.
//!
//! Wraps `model2vec-rs` with a deterministic chunk-text policy (see
//! §7 of `IDF_REPLACEMENT.md`) and a batching encode API. Loading is
//! fallible and cached: one `OnceLock` per `CodeService`, surfacing
//! `SEMANTIC_FALLBACK` when the model can't be reached.
//!
//! The canonical `minishlab/potion-base-8M` model is vendored into the
//! repository and embedded into the binary with `include_bytes!`. On first
//! use we materialize it into a shared local cache dir because
//! `model2vec-rs` currently only supports loading from real files.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use model2vec_rs::model::StaticModel;
use symslice_storage::EMBEDDING_DIM;

const MODEL_ID: &str = "potion-base-8M";
const CONFIG_JSON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/embedder/potion-base-8M/config.json"
));
const TOKENIZER_JSON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/embedder/potion-base-8M/tokenizer.json"
));
const MODEL_SAFETENSORS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/embedder/potion-base-8M/model.safetensors"
));

/// Opaque embedder handle. `Arc<Embedder>` is the shared form held on
/// `CodeService`; callers go through [`encode`] / [`encode_query`].
pub struct Embedder {
    inner: StaticModel,
}

impl Embedder {
    /// Load the built-in embedder by materializing the vendored model to a
    /// shared cache directory and pointing `model2vec-rs` at that folder.
    pub fn load() -> anyhow::Result<Self> {
        let model_dir = materialize_model_bundle()?;
        tracing::debug!(path = %model_dir.display(), "loading built-in model2vec embedder");
        let model = StaticModel::from_pretrained(&model_dir, None, None, None).map_err(|e| {
            anyhow::anyhow!("model2vec load failed from {}: {e}", model_dir.display())
        })?;
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

fn model_bundle_hash() -> &'static str {
    // Content-addressed cache key: invalidates whenever any vendored asset
    // changes. Avoids depending on the product crate's git-SHA build stamp.
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut hasher = blake3::Hasher::new();
        hasher.update(CONFIG_JSON);
        hasher.update(TOKENIZER_JSON);
        hasher.update(MODEL_SAFETENSORS);
        hasher.finalize().to_hex().as_str()[..12].to_string()
    })
}

fn materialize_model_bundle() -> anyhow::Result<PathBuf> {
    let root = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("symslice")
        .join("embedded-model")
        .join(format!("{MODEL_ID}-{}", model_bundle_hash()));
    fs::create_dir_all(&root)?;
    ensure_asset(&root.join("config.json"), CONFIG_JSON)?;
    ensure_asset(&root.join("tokenizer.json"), TOKENIZER_JSON)?;
    ensure_asset(&root.join("model.safetensors"), MODEL_SAFETENSORS)?;
    Ok(root)
}

fn ensure_asset(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let existing_len = path.metadata().ok().map(|meta| meta.len() as usize);
    if existing_len == Some(bytes.len()) {
        return Ok(());
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).or_else(|rename_error| {
        let already_materialized =
            path.metadata().ok().map(|meta| meta.len() as usize) == Some(bytes.len());
        let _ = fs::remove_file(&tmp);
        if already_materialized {
            Ok(())
        } else {
            Err(rename_error)
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn shared_embedder() -> &'static Embedder {
        static SHARED: OnceLock<Embedder> = OnceLock::new();
        SHARED.get_or_init(|| Embedder::load().unwrap())
    }

    #[test]
    fn materializes_embedded_model_bundle() {
        let root = materialize_model_bundle().unwrap();
        assert!(root.join("config.json").is_file());
        assert!(root.join("tokenizer.json").is_file());
        assert!(root.join("model.safetensors").is_file());
    }

    #[test]
    fn bundled_model_encodes_expected_dimension() {
        let embedder = shared_embedder();
        let vector = embedder.encode_query("hello world").unwrap();
        assert_eq!(vector.len(), EMBEDDING_DIM);
    }
}
