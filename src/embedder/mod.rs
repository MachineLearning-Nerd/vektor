mod onnx;
mod openai;

// Constructed by the factory in task 3.5; allow until then.
#[allow(unused_imports)]
pub use onnx::OnnxEmbedder;
// Constructed by the factory in task 3.5; allow until then.
#[allow(unused_imports)]
pub use openai::OpenAiCompatEmbedder;

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
// implemented by tasks 3.2 (ONNX) and 3.4 (OpenAI-compat), wired by 3.5 factory;
// allow dead_code until those tasks land.
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
}
