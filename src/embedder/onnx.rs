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
use crate::error::{Result, VektorError};

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

    /// Guarded access to the session for task 3.3's `embed` implementation.
    pub(crate) fn session(&self) -> &Mutex<Session> {
        &self.session
    }

    /// Tokenizer handle for task 3.3's `embed` implementation.
    pub(crate) fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
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
}
