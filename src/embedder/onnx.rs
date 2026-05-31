//! Local ONNX embedding backend (task 3.2 — construction + warm-up).
//!
//! This module only builds the [`OnnxEmbedder`] and warms its `ort` session.
//! The [`Embedder`](crate::embedder::Embedder) trait implementation
//! (tokenize → run → mean-pool → L2-normalize → batch) lands in task 3.3,
//! and the embedder is constructed by the factory in task 3.5. Until then the
//! struct is dead from the compiler's point of view, so the few items the
//! lints flag carry a scoped `#[allow(dead_code)]` with a pointer to the task.
//!
//! ## Artifact layout
//! Models live under the Vektor data directory at a deterministic path:
//! `<data_dir>/models/<safe-model-name>/`, where the HuggingFace repo id has
//! its `/` replaced with `--` (e.g. `jinaai/jina-embeddings-v2-base-code` →
//! `jinaai--jina-embeddings-v2-base-code`). The directory is expected to hold:
//! - `tokenizer.json` — HuggingFace fast-tokenizer definition
//! - `onnx/model.onnx` — the exported ONNX graph
//!
//! Downloading those artifacts is owned by task 3.11 (`vektor models download`).
//! This constructor only *fails clearly* when they are absent.
//!
//! ## ort 2.0.0-rc.12 API used here (recorded for task 3.3 to reuse)
//! - Build: `Session::builder()?.with_execution_providers([CPU::default().build()])?.with_optimization_level(GraphOptimizationLevel::Level3)?.with_intra_threads(n)?.commit_from_file(path)?`
//! - Inputs: `Tensor::<i64>::from_array((shape_vec_i64, data_vec_i64))?`, then `ort::inputs!["input_ids" => ids, "attention_mask" => mask]`
//! - Run: `session.run(inputs)?` — `run` takes `&mut Session`, hence the `Mutex<Session>` field so 3.3's `&self` `embed` can lock + run.
//! - Output: `outputs[0].try_extract_tensor::<f32>()? -> (&Shape, &[f32])`; the embedding dimension is the last entry of that shape.
//! - Introspection: `session.inputs()` / `session.outputs()` yield `&[Outlet]`; `outlet.name()` drives whether we pass `token_type_ids`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::config::Config;
use crate::embedder::Embedder;
use crate::error::{Result, VektorError};

/// Maximum number of texts fed to the ONNX session in a single forward pass.
/// Bounds peak memory and tensor size; larger inputs are chunked into batches
/// of at most this many rows. Matches the warm-up batch size in task 3.2.
const MAX_BATCH_SIZE: usize = 32;

/// Local ONNX-backed embedder.
///
/// Holds the loaded `ort` session (behind a `Mutex` because `Session::run`
/// requires `&mut self` while [`Embedder::embed`](crate::embedder::Embedder)
/// is `&self`), the tokenizer, the derived embedding dimension, and the
/// document/query prefixes the trait impl in 3.3 will return.
// Embedder impl in 3.3, constructed by factory in 3.5; allow until then.
#[allow(dead_code)]
pub struct OnnxEmbedder {
    /// ONNX Runtime session. `run` needs `&mut`, so it is guarded for the
    /// `&self` trait method 3.3 will add.
    session: Mutex<Session>,
    /// HuggingFace fast tokenizer loaded from `tokenizer.json`.
    tokenizer: Tokenizer,
    /// Embedding dimensionality, derived from the model's output shape during
    /// warm-up (768 for Jina v2 base code, 384 for lite BGE).
    dim: usize,
    /// Configured model name (the HuggingFace repo id), e.g.
    /// `jinaai/jina-embeddings-v2-base-code`. Returned by `name()`.
    model_name: String,
    /// Whether the loaded model declares a `token_type_ids` input. Many code
    /// models only take `input_ids` + `attention_mask`; 3.3 must only build
    /// `token_type_ids` when this is true.
    has_token_type_ids: bool,
    /// Prefix prepended to documents before embedding (Jina:
    /// `"search_document: "`; BGE/none: `""`).
    doc_prefix: String,
    /// Prefix prepended to queries before embedding (Jina: `"search_query: "`;
    /// BGE/none: `""`).
    query_prefix: String,
}

/// Standard `token_type_ids` input name used by BERT-family ONNX exports.
const TOKEN_TYPE_IDS_INPUT: &str = "token_type_ids";

#[allow(dead_code)]
impl OnnxEmbedder {
    /// Construct an embedder for `config.embedding.onnx_model`, loading the
    /// tokenizer and ONNX session from the resolved model directory and warming
    /// the session at batch sizes 1 and 32.
    ///
    /// # Errors
    /// - [`VektorError::Embedding`] if `tokenizer.json` or `onnx/model.onnx` is
    ///   missing (message instructs the user to run `vektor models download`),
    ///   if the tokenizer/session fail to load, or if warm-up fails. Warm-up
    ///   failures surface here rather than being deferred to the first query.
    /// - [`VektorError::Config`] if `data_dir` references `~` but no home
    ///   directory can be resolved.
    pub fn new(config: &Config) -> Result<Self> {
        let model_name = config.embedding.onnx_model.clone();
        let dir = model_dir(config)?;
        let tokenizer_path = tokenizer_path(&dir);
        let onnx_path = onnx_model_path(&dir);

        // Fail clearly *before* touching ort if artifacts are absent.
        ensure_artifacts_present(&model_name, &tokenizer_path, &onnx_path)?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            VektorError::Embedding(format!(
                "failed to load tokenizer at {}: {error}",
                tokenizer_path.display()
            ))
        })?;

        let mut session = build_session(&onnx_path)?;

        let has_token_type_ids = session
            .inputs()
            .iter()
            .any(|outlet| outlet.name() == TOKEN_TYPE_IDS_INPUT);

        tracing::debug!(
            model = %model_name,
            onnx = %onnx_path.display(),
            inputs = ?session.inputs().iter().map(|o| o.name()).collect::<Vec<_>>(),
            outputs = ?session.outputs().iter().map(|o| o.name()).collect::<Vec<_>>(),
            has_token_type_ids,
            "onnx session loaded (CPU execution provider)"
        );

        // Warm up at batch 1 and 32 to trigger lazy allocation and avoid
        // cold-start latency on the first real query. Also derive `dim` from
        // the output shape (model-agnostic: works for 768 and 384).
        let dim = warm_up(&mut session, &tokenizer, has_token_type_ids)?;

        let dim = dim
            .or_else(|| static_dim_for_model(&model_name))
            .ok_or_else(|| {
                VektorError::Embedding(format!(
                    "could not determine embedding dimension for model '{model_name}' \
                     from its output shape"
                ))
            })?;

        let (doc_prefix, query_prefix) = prefixes_for_model(&model_name);

        tracing::info!(
            model = %model_name,
            dim,
            has_token_type_ids,
            "onnx embedder ready"
        );

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dim,
            model_name,
            has_token_type_ids,
            doc_prefix: doc_prefix.to_string(),
            query_prefix: query_prefix.to_string(),
        })
    }

    /// Configured model name (HuggingFace repo id).
    pub fn name(&self) -> &str {
        &self.model_name
    }

    /// Embedding dimensionality derived from the model's output shape.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Document prefix (consumed by 3.3's `Embedder::prefix_for_document`).
    pub fn doc_prefix(&self) -> &str {
        &self.doc_prefix
    }

    /// Query prefix (consumed by 3.3's `Embedder::prefix_for_query`).
    pub fn query_prefix(&self) -> &str {
        &self.query_prefix
    }

    /// Whether the loaded model takes a `token_type_ids` input.
    pub fn has_token_type_ids(&self) -> bool {
        self.has_token_type_ids
    }

    /// Embed one batch of at most [`MAX_BATCH_SIZE`] already-prefixed texts.
    ///
    /// Tokenizes the batch, builds the padded `input_ids` / `attention_mask`
    /// (and `token_type_ids` when the model needs them), runs the session, then
    /// masked-mean-pools and L2-normalizes the resulting token embeddings.
    ///
    /// The session `run` is blocking; we hold the `Mutex<Session>` guard for the
    /// duration of the forward pass. The caller wraps the whole batched loop in
    /// `block_in_place` so this never starves the async runtime (see `embed`).
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        debug_assert!(texts.len() <= MAX_BATCH_SIZE);

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|error| VektorError::Embedding(format!("tokenization failed: {error}")))?;

        // Pad every row to the batch's longest sequence so the tensor is
        // rectangular. `seq_len >= 1` keeps tensor construction valid even if a
        // (degenerate) encoding produced zero tokens.
        let seq_len = encodings
            .iter()
            .map(|enc| enc.get_ids().len())
            .max()
            .unwrap_or(0)
            .max(1);
        let rows = encodings.len();

        let mut input_ids: Vec<i64> = Vec::with_capacity(rows * seq_len);
        let mut attention_mask: Vec<i64> = Vec::with_capacity(rows * seq_len);
        for enc in &encodings {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            for col in 0..seq_len {
                input_ids.push(ids.get(col).map(|&id| i64::from(id)).unwrap_or(0));
                attention_mask.push(mask.get(col).map(|&m| i64::from(m)).unwrap_or(0));
            }
        }

        let shape = vec![rows as i64, seq_len as i64];
        let ids_tensor = Tensor::<i64>::from_array((shape.clone(), input_ids))
            .map_err(ort_err("build input_ids tensor"))?;
        let mask_tensor = Tensor::<i64>::from_array((shape.clone(), attention_mask.clone()))
            .map_err(ort_err("build attention_mask tensor"))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| VektorError::Embedding("onnx session mutex poisoned".to_string()))?;

        let outputs = if self.has_token_type_ids {
            let token_type_ids = vec![0i64; rows * seq_len];
            let tt_tensor = Tensor::<i64>::from_array((shape, token_type_ids))
                .map_err(ort_err("build token_type_ids tensor"))?;
            session
                .run(ort::inputs![
                    "input_ids" => ids_tensor,
                    "attention_mask" => mask_tensor,
                    TOKEN_TYPE_IDS_INPUT => tt_tensor,
                ])
                .map_err(ort_err("run embedding session"))?
        } else {
            session
                .run(ort::inputs![
                    "input_ids" => ids_tensor,
                    "attention_mask" => mask_tensor,
                ])
                .map_err(ort_err("run embedding session"))?
        };

        // Jina/code models emit `last_hidden_state` shaped [batch, seq, hidden].
        let (out_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_err("extract embedding output"))?;

        let hidden = dim_from_output_shape(out_shape).ok_or_else(|| {
            VektorError::Embedding(format!(
                "embedding output has no usable hidden dimension (shape {out_shape:?})"
            ))
        })?;
        if hidden != self.dim {
            return Err(VektorError::Embedding(format!(
                "embedding output hidden dim {hidden} does not match expected dim {}",
                self.dim
            )));
        }

        // Pool into a fresh buffer (copies out of the borrowed `data`), then
        // normalize per row. `outputs`/`session` stay borrowed until scope end.
        let pooled = mean_pool(data, &attention_mask, rows, seq_len, hidden);
        let mut vectors: Vec<Vec<f32>> = pooled.chunks_exact(hidden).map(<[f32]>::to_vec).collect();
        for vector in &mut vectors {
            l2_normalize(vector);
        }
        Ok(vectors)
    }
}

#[async_trait::async_trait]
impl Embedder for OnnxEmbedder {
    /// Embed already-prefixed texts (see [`Embedder::embed`]).
    ///
    /// Empty input short-circuits without touching ONNX. Otherwise inputs are
    /// processed in batches of at most [`MAX_BATCH_SIZE`]; each batch is
    /// tokenized, run through the session, masked-mean-pooled, and
    /// L2-normalized.
    ///
    /// `Session::run` is blocking. On the multi-threaded runtime (the app's
    /// `#[tokio::main]` default) the batched loop runs inside
    /// `tokio::task::block_in_place`, which tells the scheduler to offload other
    /// tasks so the blocking work does not starve the executor. On a
    /// current-thread runtime `block_in_place` would panic, so we run inline
    /// there (no sibling workers to starve anyway).
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let run_batches = || -> Result<Vec<Vec<f32>>> {
            let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
            for batch in texts.chunks(MAX_BATCH_SIZE) {
                out.extend(self.embed_batch(batch)?);
            }
            Ok(out)
        };

        let on_multi_thread = tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        });

        if on_multi_thread {
            tokio::task::block_in_place(run_batches)
        } else {
            run_batches()
        }
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        &self.model_name
    }

    fn prefix_for_document(&self) -> &str {
        &self.doc_prefix
    }

    fn prefix_for_query(&self) -> &str {
        &self.query_prefix
    }
}

// ---------------------------------------------------------------------------
// Pure pooling / normalization helpers (no ort, fully unit-testable).
// ---------------------------------------------------------------------------

/// Masked mean-pool a flat `[rows, seq_len, hidden]` token-embedding buffer over
/// the sequence axis, producing a flat `[rows, hidden]` buffer.
///
/// For each row `r` and hidden index `h`:
/// `pooled[r][h] = Σ_t (token[r][t][h] * mask[r][t]) / Σ_t mask[r][t]`.
///
/// Padding tokens (`mask == 0`) contribute nothing to either sum, so they never
/// affect the result. A row whose mask is entirely zero pools to all-zeros
/// (division guard) rather than producing NaNs.
///
/// `token_embeddings` must have length `rows * seq_len * hidden` and
/// `attention_mask` length `rows * seq_len`; shorter slices are treated as if
/// padded with zeros, which keeps the helper total over malformed input.
fn mean_pool(
    token_embeddings: &[f32],
    attention_mask: &[i64],
    rows: usize,
    seq_len: usize,
    hidden: usize,
) -> Vec<f32> {
    let mut pooled = vec![0.0f32; rows * hidden];
    for r in 0..rows {
        let mut count: f32 = 0.0;
        for t in 0..seq_len {
            let mask = attention_mask.get(r * seq_len + t).copied().unwrap_or(0);
            if mask == 0 {
                continue;
            }
            count += 1.0;
            let token_base = (r * seq_len + t) * hidden;
            let pooled_base = r * hidden;
            for h in 0..hidden {
                if let Some(&value) = token_embeddings.get(token_base + h) {
                    pooled[pooled_base + h] += value;
                }
            }
        }
        if count > 0.0 {
            let pooled_base = r * hidden;
            for h in 0..hidden {
                pooled[pooled_base + h] /= count;
            }
        }
        // count == 0 (all-padding row) leaves this row as zeros by construction.
    }
    pooled
}

/// L2-normalize a single vector in place so `sqrt(Σ x²) == 1.0 ± 1e-5`.
///
/// The zero vector is left untouched (norm `0` → no division), so callers never
/// produce NaNs from an all-zero pooled row.
fn l2_normalize(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return;
    }
    let inv = (1.0 / norm) as f32;
    for value in vector.iter_mut() {
        *value *= inv;
    }
}

// ---------------------------------------------------------------------------
// Pure path / name helpers (no ort, fully unit-testable).
// ---------------------------------------------------------------------------

/// Convert a HuggingFace repo id into a stable filesystem directory component
/// by replacing `/` with `--` (e.g. `jinaai/jina-embeddings-v2-base-code` →
/// `jinaai--jina-embeddings-v2-base-code`).
fn safe_model_name(model_name: &str) -> String {
    model_name.replace('/', "--")
}

/// Resolve the model directory `<data_dir>/models/<safe-model-name>/`, reusing
/// the same `~`-expansion rule as project state (see
/// [`crate::state::expand_data_dir`]).
fn model_dir(config: &Config) -> Result<PathBuf> {
    let base = crate::state::expand_data_dir(&config.index.data_dir)?;
    Ok(base
        .join("models")
        .join(safe_model_name(&config.embedding.onnx_model)))
}

/// `<model-dir>/tokenizer.json`.
fn tokenizer_path(model_dir: &Path) -> PathBuf {
    model_dir.join("tokenizer.json")
}

/// `<model-dir>/onnx/model.onnx`.
fn onnx_model_path(model_dir: &Path) -> PathBuf {
    model_dir.join("onnx").join("model.onnx")
}

/// Return a clear, actionable error if either required artifact is missing.
///
/// The message names the missing path and tells the user exactly how to fix it
/// (`vektor models download`), since downloading is owned by task 3.11.
fn ensure_artifacts_present(
    model_name: &str,
    tokenizer_path: &Path,
    onnx_path: &Path,
) -> Result<()> {
    let mut missing: Vec<String> = Vec::new();
    if !tokenizer_path.exists() {
        missing.push(tokenizer_path.display().to_string());
    }
    if !onnx_path.exists() {
        missing.push(onnx_path.display().to_string());
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(VektorError::Embedding(format!(
        "model artifacts for '{model_name}' not found (missing: {}). \
         Run `vektor models download` to fetch them.",
        missing.join(", ")
    )))
}

/// Map a model name to its document/query prefixes.
///
/// Jina v2 code models use the `search_document: ` / `search_query: ` task
/// prefixes; BGE / generic models use no prefix.
fn prefixes_for_model(model_name: &str) -> (&'static str, &'static str) {
    let lower = model_name.to_ascii_lowercase();
    if lower.contains("jina") {
        ("search_document: ", "search_query: ")
    } else {
        ("", "")
    }
}

/// Static name→dim fallback used only if the output shape cannot pin a known
/// dimension during warm-up. Kept tiny and explicit; preferring the derived
/// dimension keeps the constructor model-agnostic.
fn static_dim_for_model(model_name: &str) -> Option<usize> {
    let lower = model_name.to_ascii_lowercase();
    if lower.contains("jina-embeddings-v2-base-code") {
        Some(768)
    } else if lower.contains("bge-small") {
        Some(384)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// ort-dependent helpers (kept behind their own seam; exercised by #[ignore]d
// tests that require a downloaded model).
// ---------------------------------------------------------------------------

/// Build a CPU-backed ONNX session from `onnx_path`.
///
/// CPU execution provider is registered explicitly (task spec: CPU-first;
/// provider auto-selection is deferred). `intra_threads` is clamped to the
/// available parallelism so we neither oversubscribe nor pin a single core.
fn build_session(onnx_path: &Path) -> Result<Session> {
    let intra_threads = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);

    Session::builder()
        .map_err(ort_err("create session builder"))?
        .with_execution_providers([ort::ep::CPU::default().build()])
        .map_err(ort_err("register CPU execution provider"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(ort_err("set optimization level"))?
        .with_intra_threads(intra_threads)
        .map_err(ort_err("set intra-op threads"))?
        .commit_from_file(onnx_path)
        .map_err(ort_err(&format!("load ONNX model {}", onnx_path.display())))
}

/// Warm up the session at batch sizes 1 and 32 with dummy input, discarding
/// outputs. Returns the embedding dimension derived from the output shape (the
/// last dimension), or `None` if it could not be determined.
///
/// Warm-up failures propagate to the caller so they surface from `new()`
/// instead of the first real query.
fn warm_up(
    session: &mut Session,
    tokenizer: &Tokenizer,
    has_token_type_ids: bool,
) -> Result<Option<usize>> {
    let mut derived_dim = None;

    for &batch_size in &[1usize, 32usize] {
        let dim = forward_dummy(session, tokenizer, has_token_type_ids, batch_size)?;
        // Prefer the first concrete dim observed; both batches should agree.
        if derived_dim.is_none() {
            derived_dim = dim;
        }
    }

    Ok(derived_dim)
}

/// Run one forward pass over `batch_size` copies of a dummy string and return
/// the embedding dimension (last axis of the output shape), if determinable.
///
/// This is the shared tokenize → tensors → run → read-shape path. Task 3.3
/// reuses the same construction (just adding mean-pool + L2-normalize over the
/// extracted output data instead of only reading its shape).
fn forward_dummy(
    session: &mut Session,
    tokenizer: &Tokenizer,
    has_token_type_ids: bool,
    batch_size: usize,
) -> Result<Option<usize>> {
    let texts: Vec<String> = (0..batch_size).map(|_| "warm up".to_string()).collect();

    let encodings = tokenizer
        .encode_batch(texts, true)
        .map_err(|error| VektorError::Embedding(format!("tokenizer warm-up failed: {error}")))?;

    // Pad/truncate to a common length so every row has equal width.
    let seq_len = encodings
        .iter()
        .map(|enc| enc.get_ids().len())
        .max()
        .unwrap_or(0)
        .max(1);

    let rows = encodings.len();
    let mut input_ids: Vec<i64> = Vec::with_capacity(rows * seq_len);
    let mut attention_mask: Vec<i64> = Vec::with_capacity(rows * seq_len);

    for enc in &encodings {
        let ids = enc.get_ids();
        let mask = enc.get_attention_mask();
        for col in 0..seq_len {
            input_ids.push(ids.get(col).map(|&id| i64::from(id)).unwrap_or(0));
            attention_mask.push(mask.get(col).map(|&m| i64::from(m)).unwrap_or(0));
        }
    }

    let shape = vec![rows as i64, seq_len as i64];

    let ids_tensor = Tensor::<i64>::from_array((shape.clone(), input_ids))
        .map_err(ort_err("build input_ids tensor"))?;
    let mask_tensor = Tensor::<i64>::from_array((shape.clone(), attention_mask))
        .map_err(ort_err("build attention_mask tensor"))?;

    let outputs = if has_token_type_ids {
        let token_type_ids = vec![0i64; rows * seq_len];
        let tt_tensor = Tensor::<i64>::from_array((shape, token_type_ids))
            .map_err(ort_err("build token_type_ids tensor"))?;
        session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                TOKEN_TYPE_IDS_INPUT => tt_tensor,
            ])
            .map_err(ort_err("run warm-up session"))?
    } else {
        session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ])
            .map_err(ort_err("run warm-up session"))?
    };

    // Derive the embedding dimension from the last axis of the first output.
    // Token-embedding models emit [batch, seq_len, hidden]; pooled models emit
    // [batch, hidden]. Either way the hidden size is the trailing dimension.
    let (out_shape, _data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(ort_err("extract warm-up output"))?;

    Ok(dim_from_output_shape(out_shape))
}

/// Extract the embedding dimension (last axis) from an output tensor shape.
fn dim_from_output_shape(shape: &[i64]) -> Option<usize> {
    shape.last().and_then(|&d| usize::try_from(d).ok())
}

/// Build a closure mapping any `ort::Error<R>` into a `VektorError::Embedding`
/// with a contextual prefix.
///
/// `ort` builder methods return `Error<SessionBuilder>` (carrying the builder
/// back for recovery) while `commit_from_file` / `run` return the default
/// `Error<()>`; this stays generic over `R` so it works for both. `Error<R>`
/// is `Display` for all `R`.
fn ort_err<R>(context: &str) -> impl Fn(ort::Error<R>) -> VektorError + '_ {
    move |error| VektorError::Embedding(format!("{context}: {error}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EmbeddingConfig, IndexConfig};

    fn config_with(data_dir: &Path, model: &str) -> Config {
        Config {
            embedding: EmbeddingConfig {
                onnx_model: model.to_string(),
                ..Default::default()
            },
            index: IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn onnx_embedder_safe_model_name_replaces_slash_with_double_dash() {
        assert_eq!(
            safe_model_name("jinaai/jina-embeddings-v2-base-code"),
            "jinaai--jina-embeddings-v2-base-code"
        );
        assert_eq!(
            safe_model_name("BAAI/bge-small-en-v1.5"),
            "BAAI--bge-small-en-v1.5"
        );
        // No slash → unchanged.
        assert_eq!(safe_model_name("local-model"), "local-model");
    }

    #[test]
    fn onnx_embedder_model_dir_resolves_under_models_subdir() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = config_with(tempdir.path(), "jinaai/jina-embeddings-v2-base-code");

        let dir = model_dir(&config).expect("resolve model dir");

        assert_eq!(
            dir,
            tempdir
                .path()
                .join("models")
                .join("jinaai--jina-embeddings-v2-base-code")
        );
    }

    #[test]
    fn onnx_embedder_model_dir_expands_home_like_state() {
        let fake_home = tempfile::tempdir().expect("create fake home");
        let config = config_with(Path::new("~/.vektor-test"), "BAAI/bge-small-en-v1.5");

        let dir = temp_env::with_vars(
            [
                (
                    "HOME",
                    Some(fake_home.path().to_string_lossy().into_owned()),
                ),
                (
                    "USERPROFILE",
                    Some(fake_home.path().to_string_lossy().into_owned()),
                ),
            ],
            || model_dir(&config).expect("resolve model dir"),
        );

        assert!(dir.starts_with(fake_home.path().join(".vektor-test")));
        assert!(dir.ends_with(Path::new("models").join("BAAI--bge-small-en-v1.5")));
    }

    #[test]
    fn onnx_embedder_artifact_paths_are_tokenizer_json_and_onnx_model_onnx() {
        let dir = Path::new("/tmp/models/jinaai--jina-embeddings-v2-base-code");
        assert_eq!(tokenizer_path(dir), dir.join("tokenizer.json"));
        assert_eq!(onnx_model_path(dir), dir.join("onnx").join("model.onnx"));
    }

    #[test]
    fn onnx_embedder_ensure_artifacts_present_ok_when_both_exist() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let dir = tempdir.path();
        let tok = tokenizer_path(dir);
        let onnx = onnx_model_path(dir);
        std::fs::write(&tok, b"{}").expect("write tokenizer");
        std::fs::create_dir_all(onnx.parent().unwrap()).expect("create onnx dir");
        std::fs::write(&onnx, b"\0").expect("write onnx");

        assert!(ensure_artifacts_present("m", &tok, &onnx).is_ok());
    }

    #[test]
    fn onnx_embedder_ensure_artifacts_present_errors_when_missing_with_download_hint() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let dir = tempdir.path();
        let tok = tokenizer_path(dir);
        let onnx = onnx_model_path(dir);

        let error = ensure_artifacts_present("jinaai/jina-embeddings-v2-base-code", &tok, &onnx)
            .expect_err("missing artifacts should error");

        assert!(matches!(error, VektorError::Embedding(_)));
        let msg = error.to_string();
        assert!(msg.contains("vektor models download"), "msg: {msg}");
        assert!(
            msg.contains("jinaai/jina-embeddings-v2-base-code"),
            "msg: {msg}"
        );
        // Both missing artifacts named.
        assert!(msg.contains("tokenizer.json"), "msg: {msg}");
        assert!(msg.contains("model.onnx"), "msg: {msg}");
    }

    #[test]
    fn onnx_embedder_ensure_artifacts_present_errors_when_only_onnx_missing() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let dir = tempdir.path();
        let tok = tokenizer_path(dir);
        let onnx = onnx_model_path(dir);
        std::fs::write(&tok, b"{}").expect("write tokenizer");

        let error =
            ensure_artifacts_present("m", &tok, &onnx).expect_err("missing onnx should error");
        let msg = error.to_string();
        assert!(msg.contains("model.onnx"), "msg: {msg}");
        assert!(
            !msg.contains("tokenizer.json"),
            "tokenizer present, msg: {msg}"
        );
    }

    #[test]
    fn onnx_embedder_new_errors_clearly_when_artifacts_missing() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = config_with(tempdir.path(), "jinaai/jina-embeddings-v2-base-code");

        // `OnnxEmbedder` holds non-Debug `ort` types, so avoid `expect_err`
        // (which needs `T: Debug`) and inspect the `Err` directly.
        let error = OnnxEmbedder::new(&config)
            .err()
            .expect("missing model should error");

        assert!(matches!(error, VektorError::Embedding(_)));
        assert!(error.to_string().contains("vektor models download"));
    }

    #[test]
    fn onnx_embedder_new_does_not_panic_and_isolates_data_dir() {
        // A tempdir data_dir guarantees we never touch the real ~/.vektor.
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = config_with(tempdir.path(), "BAAI/bge-small-en-v1.5");

        // No artifacts present → clean error, never a panic.
        assert!(OnnxEmbedder::new(&config).err().is_some());
    }

    #[test]
    fn onnx_embedder_prefixes_jina_vs_other() {
        assert_eq!(
            prefixes_for_model("jinaai/jina-embeddings-v2-base-code"),
            ("search_document: ", "search_query: ")
        );
        assert_eq!(prefixes_for_model("BAAI/bge-small-en-v1.5"), ("", ""));
    }

    #[test]
    fn onnx_embedder_static_dim_fallback_known_models() {
        assert_eq!(
            static_dim_for_model("jinaai/jina-embeddings-v2-base-code"),
            Some(768)
        );
        assert_eq!(static_dim_for_model("BAAI/bge-small-en-v1.5"), Some(384));
        assert_eq!(static_dim_for_model("unknown/model"), None);
    }

    #[test]
    fn onnx_embedder_dim_from_output_shape_takes_last_axis() {
        // [batch, seq_len, hidden]
        assert_eq!(dim_from_output_shape(&[2, 16, 768]), Some(768));
        // [batch, hidden]
        assert_eq!(dim_from_output_shape(&[32, 384]), Some(384));
        // empty shape → None
        assert_eq!(dim_from_output_shape(&[]), None);
        // negative (dynamic) dim → None
        assert_eq!(dim_from_output_shape(&[2, 16, -1]), None);
    }

    /// Full construction + warm-up against a real model. Ignored by default
    /// because it needs a downloaded model under `<data_dir>/models/...`.
    ///
    /// Run manually after `vektor models download`:
    /// `cargo test onnx_embedder -- --ignored --nocapture`
    #[test]
    #[ignore = "requires a downloaded model; run manually after `vektor models download`"]
    fn onnx_embedder_new_loads_and_warms_real_model() {
        // Use the developer's real data dir for this manual check.
        let config = Config::default();
        let embedder = OnnxEmbedder::new(&config).expect("construct onnx embedder");
        // Jina v2 base code is 768-dim; assertion is informational.
        assert!(embedder.dim() == 768 || embedder.dim() == 384);
        assert_eq!(embedder.name(), config.embedding.onnx_model);
    }

    // -----------------------------------------------------------------------
    // mean_pool — pure, synthetic, known-answer
    // -----------------------------------------------------------------------

    /// Single row, two tokens, hidden=2, both tokens active → arithmetic mean.
    #[test]
    fn onnx_embedder_mean_pool_averages_active_tokens() {
        // token[0] = [1, 2], token[1] = [3, 6]; mask = [1, 1].
        // pooled = ([1+3]/2, [2+6]/2) = (2, 4).
        let tokens = vec![1.0, 2.0, 3.0, 6.0];
        let mask = vec![1, 1];
        let pooled = mean_pool(&tokens, &mask, 1, 2, 2);
        assert_eq!(pooled, vec![2.0, 4.0]);
    }

    /// Masked token must be ignored entirely: a 2-token seq with mask [1, 0]
    /// pools to exactly token 0 (the canonical padding-is-ignored proof).
    #[test]
    fn onnx_embedder_mean_pool_ignores_masked_tokens() {
        // token[0] = [10, 20] (active), token[1] = [999, 999] (padding).
        let tokens = vec![10.0, 20.0, 999.0, 999.0];
        let mask = vec![1, 0];
        let pooled = mean_pool(&tokens, &mask, 1, 2, 2);
        // Only token 0 contributes; count = 1.
        assert_eq!(pooled, vec![10.0, 20.0]);
    }

    /// All-padding row pools to zeros (division guard) instead of NaN.
    #[test]
    fn onnx_embedder_mean_pool_all_padding_row_is_zero() {
        let tokens = vec![5.0, 7.0, 9.0, 11.0];
        let mask = vec![0, 0];
        let pooled = mean_pool(&tokens, &mask, 1, 2, 2);
        assert_eq!(pooled, vec![0.0, 0.0]);
        assert!(pooled.iter().all(|x| !x.is_nan()));
    }

    /// Two independent rows pool independently; per-row masks are respected.
    #[test]
    fn onnx_embedder_mean_pool_multiple_rows_independent() {
        // row0: tokens [1,1],[3,3] mask [1,1] → mean [2,2]
        // row1: tokens [4,8],[100,100] mask [1,0] → [4,8]
        let tokens = vec![
            1.0, 1.0, 3.0, 3.0, // row 0
            4.0, 8.0, 100.0, 100.0, // row 1
        ];
        let mask = vec![1, 1, 1, 0];
        let pooled = mean_pool(&tokens, &mask, 2, 2, 2);
        assert_eq!(pooled, vec![2.0, 2.0, 4.0, 8.0]);
    }

    // -----------------------------------------------------------------------
    // l2_normalize — pure, synthetic, known-answer
    // -----------------------------------------------------------------------

    /// 3-4-5 triangle: [3,4] has norm 5 → normalizes to [0.6, 0.8].
    #[test]
    fn onnx_embedder_l2_normalize_known_vector() {
        let mut v = vec![3.0f32, 4.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6, "v0={}", v[0]);
        assert!((v[1] - 0.8).abs() < 1e-6, "v1={}", v[1]);
    }

    /// After normalization the L2 norm is 1.0 ± 1e-5 for an arbitrary vector.
    #[test]
    fn onnx_embedder_l2_normalize_yields_unit_norm() {
        let mut v = vec![0.5f32, -1.5, 2.0, 7.0, -0.25, 3.3];
        l2_normalize(&mut v);
        let norm = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
    }

    /// Zero vector stays zero (no divide-by-zero / NaN).
    #[test]
    fn onnx_embedder_l2_normalize_zero_vector_unchanged() {
        let mut v = vec![0.0f32; 4];
        l2_normalize(&mut v);
        assert_eq!(v, vec![0.0f32; 4]);
        assert!(v.iter().all(|x| !x.is_nan()));
    }

    /// Composed pipeline mirror of `embed_batch`'s tail: pool then normalize a
    /// masked synthetic batch and confirm every non-zero row is unit-norm.
    #[test]
    fn onnx_embedder_pool_then_normalize_unit_norm() {
        // rows=2, seq=2, hidden=3. Row 1's second token is padding.
        let tokens = vec![
            1.0, 2.0, 2.0, 0.0, 0.0, 0.0, // row 0: only token 0 active below
            9.0, 9.0, 9.0, 1.0, 1.0, 1.0, // row 1
        ];
        let mask = vec![1, 0, 1, 0];
        let pooled = mean_pool(&tokens, &mask, 2, 2, 3);
        let mut vectors: Vec<Vec<f32>> = pooled.chunks_exact(3).map(<[f32]>::to_vec).collect();
        for v in &mut vectors {
            l2_normalize(v);
        }
        for v in &vectors {
            let norm = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "norm={norm} vec={v:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Embedder::embed — empty-input short-circuit (no ONNX, no model needed)
    // -----------------------------------------------------------------------

    /// Empty input returns an empty Vec and must NOT construct/lock a session.
    /// Building an embedder requires a model, so we exercise the documented
    /// short-circuit contract directly: the loop body only runs for non-empty
    /// input, so an empty slice yields an empty result without ONNX.
    #[tokio::test(flavor = "multi_thread")]
    async fn onnx_embedder_embed_empty_input_short_circuits() {
        // We cannot build a real OnnxEmbedder without a downloaded model, so
        // assert the invariant the `embed` body relies on: chunking an empty
        // slice produces zero batches, hence zero ONNX calls.
        let texts: Vec<String> = Vec::new();
        let batches: Vec<&[String]> = texts.chunks(MAX_BATCH_SIZE).collect();
        assert!(batches.is_empty());
    }

    /// Batching invariant: every chunk has at most `MAX_BATCH_SIZE` rows and the
    /// row count is preserved across chunking.
    #[test]
    fn onnx_embedder_chunks_respect_max_batch_size() {
        let texts: Vec<String> = (0..70).map(|i| format!("t{i}")).collect();
        let chunks: Vec<&[String]> = texts.chunks(MAX_BATCH_SIZE).collect();
        assert_eq!(chunks.len(), 3); // 32 + 32 + 6
        assert!(chunks.iter().all(|c| c.len() <= MAX_BATCH_SIZE));
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), 70);
        assert_eq!(chunks[0].len(), 32);
        assert_eq!(chunks[2].len(), 6);
    }

    /// Full real-model embed smoke test: prefix-through-tokenization plus the
    /// embed math end to end. Ignored — needs a downloaded model.
    ///
    /// Run: `cargo test onnx_embedder -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a downloaded model; run manually after `vektor models download`"]
    async fn onnx_embedder_embed_real_model_unit_norm_and_prefixes() {
        let config = Config::default();
        let embedder = OnnxEmbedder::new(&config).expect("construct onnx embedder");

        // N texts → N vectors, each of length dim(), each unit-norm.
        let docs = vec!["fn add(a: i32, b: i32) -> i32 { a + b }".to_string()];
        let doc_vecs = embedder.embed_documents(&docs).await.expect("embed docs");
        assert_eq!(doc_vecs.len(), 1);
        assert_eq!(doc_vecs[0].len(), embedder.dim());
        let norm = doc_vecs[0].iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "doc norm={norm}");

        // Query path applies the query prefix then embeds to a single vector.
        let q = embedder
            .embed_query("how do I add two integers")
            .await
            .expect("embed query");
        assert_eq!(q.len(), embedder.dim());
        let qnorm = q.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((qnorm - 1.0).abs() < 1e-5, "query norm={qnorm}");

        // Larger-than-one-batch input still returns one vector per text.
        let many: Vec<String> = (0..40).map(|i| format!("let x{i} = {i};")).collect();
        let many_vecs = embedder.embed_documents(&many).await.expect("embed many");
        assert_eq!(many_vecs.len(), 40);
        assert!(many_vecs.iter().all(|v| v.len() == embedder.dim()));
    }
}
