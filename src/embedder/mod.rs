mod onnx;
mod openai;

pub use onnx::OnnxEmbedder;
pub use openai::OpenAiCompatEmbedder;

use crate::config::Config;
use crate::error::{Result, VektorError};

/// Shared async embedding backend contract.
///
/// The trait owns document/query prefix handling (Jina v2 `"search_document: "` /
/// `"search_query: "` strategy) so callers cannot accidentally embed code chunks
/// and search queries with the wrong prefix.  Backends that do not need task
/// prefixes simply return `""` from both prefix methods.
///
/// # Object safety
/// `embed` takes `&self` and returns a boxed future via `async_trait`, making
/// the trait object-safe and usable behind `Box<dyn Embedder>`.
///
/// # Usage
/// Prefer the provided helpers `embed_documents` / `embed_query` over calling
/// `embed` directly — they guarantee the correct prefix is applied exactly once.
// trait methods used by factory return type Box<dyn Embedder>; trait itself has
// no external caller until 3.7b; allow dead_code on the trait methods until then.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a batch of raw texts (no prefix manipulation).
    ///
    /// This is the **backend-facing** entry point.  Callers outside this module
    /// should use `embed_documents` or `embed_query` instead.
    ///
    /// Returns one embedding vector per input text in the same order.
    async fn embed(&self, texts: &[String]) -> crate::error::Result<Vec<Vec<f32>>>;

    /// Dimensionality of the vectors produced by this backend.
    fn dim(&self) -> usize;

    /// Human-readable model identifier (e.g. `"jina-embeddings-v2-base-code"`).
    fn name(&self) -> &str;

    /// Prefix to prepend to every document (chunk) before embedding.
    ///
    /// Jina v2 code: `"search_document: "`.  Backends without task prefixes: `""`.
    fn prefix_for_document(&self) -> &str;

    /// Prefix to prepend to a search query before embedding.
    ///
    /// Jina v2 code: `"search_query: "`.  Backends without task prefixes: `""`.
    fn prefix_for_query(&self) -> &str;

    // ------------------------------------------------------------------
    // Provided helpers — apply the correct prefix, then delegate to embed.
    // These are the public surface callers should use.
    // ------------------------------------------------------------------

    /// Embed a batch of document texts (e.g. code chunks).
    ///
    /// Prepends `prefix_for_document()` to every element exactly once before
    /// calling the backend.  Empty prefixes leave the text unchanged.
    async fn embed_documents(&self, texts: &[String]) -> crate::error::Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| {
                let p = self.prefix_for_document();
                if p.is_empty() {
                    t.clone()
                } else {
                    format!("{p}{t}")
                }
            })
            .collect();
        self.embed(&prefixed).await
    }

    /// Embed a single search query.
    ///
    /// Prepends `prefix_for_query()` to the query exactly once before calling
    /// the backend.  Empty prefixes leave the text unchanged.
    ///
    /// Returns the single embedding vector as `Vec<f32>`.
    async fn embed_query(&self, query: &str) -> crate::error::Result<Vec<f32>> {
        let p = self.prefix_for_query();
        let prefixed = if p.is_empty() {
            query.to_owned()
        } else {
            format!("{p}{query}")
        };
        let mut results = self.embed(&[prefixed]).await?;
        // embed() must return one vector per input; return a recoverable error
        // instead of panicking if the backend returns an empty Vec.
        results.pop().ok_or_else(|| {
            crate::error::VektorError::Embedding(
                "embedder returned no vector for query".to_string(),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Build the configured embedding backend and return it as `Box<dyn Embedder>`.
///
/// Reads `config.embedding.backend` to select among supported backends:
///
/// - `"onnx"` → constructs [`OnnxEmbedder`]; fails clearly when model artifacts
///   are absent (instructs the user to run `vektor models download`).
/// - `"openai"` → constructs [`OpenAiCompatEmbedder`]; if construction fails
///   **and** `config.embedding.fallback_to_onnx == true`, falls back to ONNX
///   once and logs the fallback at `warn` level.  If `fallback_to_onnx` is
///   `false` the original OpenAI error is returned unchanged.
/// - `"ollama"` → returns a clear [`VektorError::Config`] stating that the
///   Ollama backend is deferred (not a silent fallback).
/// - any other value → returns a clear [`VektorError::Config`] (unsupported).
///
/// The factory has no knowledge of indexing, vector storage, or CLI concerns;
/// those layers call this function and receive a ready-to-use embedder.
///
/// # Errors
/// - [`VektorError::Config`] for unsupported or deferred backends.
/// - [`VektorError::Embedding`] when ONNX artifact files are missing.
/// - The original backend error when `fallback_to_onnx` is false and OpenAI
///   construction fails.
// called by index pipeline in 3.7b/3.7c; allow until then.
#[allow(dead_code)]
pub fn build_embedder(config: &Config) -> Result<Box<dyn Embedder>> {
    let backend = config.embedding.backend.as_str();

    match backend {
        "onnx" => {
            let embedder = OnnxEmbedder::new(config)?;
            Ok(Box::new(embedder))
        }
        "openai" => match OpenAiCompatEmbedder::new(config) {
            Ok(embedder) => Ok(Box::new(embedder)),
            Err(openai_err) => {
                if config.embedding.fallback_to_onnx {
                    tracing::warn!(
                        error = %openai_err,
                        "OpenAI embedder initialization failed; \
                         falling back to ONNX (fallback_to_onnx = true)"
                    );
                    let embedder = OnnxEmbedder::new(config)?;
                    Ok(Box::new(embedder))
                } else {
                    Err(openai_err)
                }
            }
        },
        "ollama" => Err(VektorError::Config(
            "the Ollama embedding backend is deferred (not yet implemented); \
             set backend = \"onnx\" for local inference or backend = \"openai\" \
             for a cloud-compatible endpoint"
                .to_string(),
        )),
        other => Err(VektorError::Config(format!(
            "unsupported embedding backend: \"{other}\"; \
             supported values are \"onnx\" and \"openai\""
        ))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Fake embedder that records every raw text passed to `embed()` and
    /// returns zero vectors of the configured dimension.  Used to assert that
    /// prefixes are applied exactly once.
    struct FakeEmbedder {
        dim: usize,
        doc_prefix: &'static str,
        query_prefix: &'static str,
        /// Texts received accumulated across all `embed()` calls.
        received: Mutex<Vec<String>>,
    }

    impl FakeEmbedder {
        fn new(dim: usize, doc_prefix: &'static str, query_prefix: &'static str) -> Self {
            Self {
                dim,
                doc_prefix,
                query_prefix,
                received: Mutex::new(Vec::new()),
            }
        }

        fn received_texts(&self) -> Vec<String> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> crate::error::Result<Vec<Vec<f32>>> {
            self.received.lock().unwrap().extend_from_slice(texts);
            Ok(texts.iter().map(|_| vec![0.0f32; self.dim]).collect())
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn name(&self) -> &str {
            "fake-embedder"
        }

        fn prefix_for_document(&self) -> &str {
            self.doc_prefix
        }

        fn prefix_for_query(&self) -> &str {
            self.query_prefix
        }
    }

    // -----------------------------------------------------------------------
    // embed_documents — Jina-style prefix
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn embed_documents_jina_prefix_prepended_once() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");
        let texts = vec!["fn foo() {}".to_owned(), "struct Bar;".to_owned()];

        let vecs = embedder.embed_documents(&texts).await.unwrap();

        // Two vectors returned, one per input.
        assert_eq!(vecs.len(), 2);

        let received = embedder.received_texts();
        assert_eq!(received.len(), 2);
        // Prefix applied exactly once.
        assert_eq!(received[0], "search_document: fn foo() {}");
        assert_eq!(received[1], "search_document: struct Bar;");
        // Original texts are unmodified.
        assert_eq!(texts[0], "fn foo() {}");
    }

    #[tokio::test]
    async fn embed_documents_returns_one_vector_per_text() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");
        let texts: Vec<String> = (0..5).map(|i| format!("chunk {i}")).collect();

        let vecs = embedder.embed_documents(&texts).await.unwrap();

        assert_eq!(vecs.len(), 5);
        for v in &vecs {
            assert_eq!(v.len(), 768);
        }
    }

    // -----------------------------------------------------------------------
    // embed_query — Jina-style prefix
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn embed_query_jina_prefix_prepended_once() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");

        let vec = embedder.embed_query("what does foo return?").await.unwrap();

        assert_eq!(vec.len(), 768);
        let received = embedder.received_texts();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0], "search_query: what does foo return?");
    }

    #[tokio::test]
    async fn embed_query_returns_single_flat_vector() {
        let embedder = FakeEmbedder::new(384, "search_document: ", "search_query: ");
        let vec = embedder.embed_query("hello world").await.unwrap();
        assert_eq!(vec.len(), 384);
    }

    // -----------------------------------------------------------------------
    // Empty-prefix backend — no stray prefix added
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn embed_documents_empty_prefix_leaves_text_unchanged() {
        let embedder = FakeEmbedder::new(1536, "", "");
        let texts = vec!["some code".to_owned()];

        embedder.embed_documents(&texts).await.unwrap();

        let received = embedder.received_texts();
        assert_eq!(received[0], "some code");
    }

    #[tokio::test]
    async fn embed_query_empty_prefix_leaves_query_unchanged() {
        let embedder = FakeEmbedder::new(1536, "", "");

        embedder.embed_query("my query").await.unwrap();

        let received = embedder.received_texts();
        assert_eq!(received[0], "my query");
    }

    // -----------------------------------------------------------------------
    // Object-safety — usable behind Box<dyn Embedder>
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn trait_object_embed_documents() {
        let embedder: Box<dyn Embedder> = Box::new(FakeEmbedder::new(
            768,
            "search_document: ",
            "search_query: ",
        ));
        let texts = vec!["trait object test".to_owned()];

        let vecs = embedder.embed_documents(&texts).await.unwrap();

        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), 768);
    }

    #[tokio::test]
    async fn trait_object_embed_query() {
        let embedder: Box<dyn Embedder> = Box::new(FakeEmbedder::new(
            768,
            "search_document: ",
            "search_query: ",
        ));

        let vec = embedder.embed_query("trait object query").await.unwrap();

        assert_eq!(vec.len(), 768);
    }

    // -----------------------------------------------------------------------
    // dim / name accessors
    // -----------------------------------------------------------------------

    #[test]
    fn dim_and_name_accessible() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");
        assert_eq!(embedder.dim(), 768);
        assert_eq!(embedder.name(), "fake-embedder");
    }

    // -----------------------------------------------------------------------
    // Prefix methods exposed on the trait
    // -----------------------------------------------------------------------

    #[test]
    fn prefix_methods_return_correct_values_jina() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");
        assert_eq!(embedder.prefix_for_document(), "search_document: ");
        assert_eq!(embedder.prefix_for_query(), "search_query: ");
    }

    #[test]
    fn prefix_methods_return_empty_for_no_prefix_backend() {
        let embedder = FakeEmbedder::new(1536, "", "");
        assert_eq!(embedder.prefix_for_document(), "");
        assert_eq!(embedder.prefix_for_query(), "");
    }

    // -----------------------------------------------------------------------
    // Prefix applied only once — not double-prefixed on repeated calls
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn embed_documents_prefix_not_double_applied_across_calls() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");
        let texts = vec!["alpha".to_owned()];

        // First call.
        embedder.embed_documents(&texts).await.unwrap();
        // Second independent call with the same input — should still have exactly
        // one prefix per call, not accumulate double prefixes.
        embedder.embed_documents(&texts).await.unwrap();

        let received = embedder.received_texts();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0], "search_document: alpha");
        assert_eq!(received[1], "search_document: alpha");
        // If the prefix had been applied to the original slice in-place (it isn't),
        // the second call would produce "search_document: search_document: alpha".
        assert!(!received[1].starts_with("search_document: search_document:"));
    }

    #[tokio::test]
    async fn embed_query_prefix_not_double_applied_across_calls() {
        let embedder = FakeEmbedder::new(768, "search_document: ", "search_query: ");

        // First call.
        embedder.embed_query("alpha").await.unwrap();
        // Second independent call with the same input — should still have exactly
        // one prefix per call, not accumulate double prefixes.
        embedder.embed_query("alpha").await.unwrap();

        let received = embedder.received_texts();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0], "search_query: alpha");
        assert_eq!(received[1], "search_query: alpha");
        // If the prefix had been applied to the original text in-place (it isn't),
        // the second call would produce "search_query: search_query: alpha".
        assert!(!received[1].starts_with("search_query: search_query:"));
    }

    // -----------------------------------------------------------------------
    // build_embedder — factory routing tests
    //
    // OnnxEmbedder::new requires real model artifacts on disk (none in CI), so
    // tests assert on ROUTING DECISIONS rather than successful ONNX construction:
    //
    // - backend="onnx"    → error is Embedding (vektor models download hint)
    // - backend="openai" + empty key + fallback=false → Config error (OpenAI key)
    // - backend="openai" + empty key + fallback=true  → Embedding error (ONNX path)
    // - backend="ollama"  → Config error (deferred message)
    // - backend="zzz"     → Config error (unsupported message)
    //
    // HOME/data_dir is isolated via tempdir so real ~/.vektor models are never found.
    // -----------------------------------------------------------------------

    use crate::config::{EmbeddingConfig, IndexConfig};

    fn factory_config(
        backend: &str,
        openai_key: &str,
        fallback: bool,
        data_dir: &std::path::Path,
    ) -> Config {
        Config {
            embedding: EmbeddingConfig {
                backend: backend.to_string(),
                openai_api_key: openai_key.to_string(),
                fallback_to_onnx: fallback,
                ..EmbeddingConfig::default()
            },
            index: IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..IndexConfig::default()
            },
            ..Config::default()
        }
    }

    /// backend="onnx" with no model on disk → routes to ONNX → Embedding error
    /// with a hint to run `vektor models download`.
    #[test]
    fn build_embedder_onnx_backend_routes_to_onnx_and_errors_on_missing_model() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = factory_config("onnx", "", true, tempdir.path());

        // Box<dyn Embedder> is not Debug, so avoid expect_err (which needs T: Debug).
        let err = build_embedder(&config)
            .err()
            .expect("no model → must error");

        assert!(
            matches!(err, VektorError::Embedding(_)),
            "expected Embedding error, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("vektor models download"),
            "error must hint at download: {msg}"
        );
    }

    /// backend="openai" + empty key + fallback=false → OpenAI Config error returned.
    #[test]
    fn build_embedder_openai_empty_key_no_fallback_returns_openai_config_error() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = factory_config("openai", "", false, tempdir.path());

        // Box<dyn Embedder> is not Debug; use .err().expect instead of expect_err.
        let err = build_embedder(&config)
            .err()
            .expect("empty key → must error");

        assert!(
            matches!(err, VektorError::Config(_)),
            "expected Config error (openai key), got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("openai_api_key"),
            "error must mention the missing key field: {msg}"
        );
    }

    /// backend="openai" + empty key + fallback=true → falls back to ONNX →
    /// Embedding error (not the OpenAI Config error), proving the fallback path ran.
    #[test]
    fn build_embedder_openai_empty_key_with_fallback_attempts_onnx() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = factory_config("openai", "", true, tempdir.path());

        // Box<dyn Embedder> is not Debug; use .err().expect instead of expect_err.
        let err = build_embedder(&config)
            .err()
            .expect("fallback to ONNX → must error (no model)");

        // If we got Config(openai_api_key …) the fallback never ran; we must see Embedding.
        assert!(
            matches!(err, VektorError::Embedding(_)),
            "expected Embedding error (ONNX path taken), got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("vektor models download"),
            "ONNX path must produce the download hint: {msg}"
        );
    }

    /// backend="ollama" → clear deferred-backend Config error, not a silent fallback.
    #[test]
    fn build_embedder_ollama_returns_deferred_config_error() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = factory_config("ollama", "", true, tempdir.path());

        // Box<dyn Embedder> is not Debug; use .err().expect instead of expect_err.
        let err = build_embedder(&config).err().expect("ollama must error");

        assert!(
            matches!(err, VektorError::Config(_)),
            "expected Config error for ollama, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("deferred") || msg.to_lowercase().contains("not yet"),
            "error must mention deferred/not yet: {msg}"
        );
    }

    /// backend="zzz" (unknown) → clear unsupported-backend Config error.
    #[test]
    fn build_embedder_unsupported_backend_returns_config_error() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = factory_config("zzz", "", false, tempdir.path());

        // Box<dyn Embedder> is not Debug; use .err().expect instead of expect_err.
        let err = build_embedder(&config)
            .err()
            .expect("unknown backend must error");

        assert!(
            matches!(err, VektorError::Config(_)),
            "expected Config error for unknown backend, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("zzz"),
            "error must echo the unsupported value: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("unsupported"),
            "error must say unsupported: {msg}"
        );
    }
}
