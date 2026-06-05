use std::time::Instant;

use crate::{embedder::Embedder, error::Result};

const SINGLE_BATCH_SIZE: usize = 1;
const MAX_BATCH_SIZE: usize = 32;
const DUMMY_TEXT: &str = "vektor warmup";

pub(crate) struct WarmUp;

impl WarmUp {
    pub(crate) async fn run(embedder: &dyn Embedder) -> Result<()> {
        let started_at = Instant::now();
        Self::embed_dummy_batch(embedder, SINGLE_BATCH_SIZE).await?;
        Self::embed_dummy_batch(embedder, MAX_BATCH_SIZE).await?;
        tracing::info!(
            elapsed_ms = started_at.elapsed().as_millis(),
            model = embedder.name(),
            dim = embedder.dim(),
            "embedder warm-up complete"
        );
        Ok(())
    }

    async fn embed_dummy_batch(embedder: &dyn Embedder, batch_size: usize) -> Result<()> {
        let texts = (0..batch_size)
            .map(|index| format!("{DUMMY_TEXT} {index}"))
            .collect::<Vec<_>>();
        let _vectors = embedder.embed_documents(&texts).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{embedder::Embedder, error::VektorError};
    use std::sync::Mutex;

    struct RecordingEmbedder {
        calls: Mutex<Vec<Vec<String>>>,
        fail: bool,
    }

    impl RecordingEmbedder {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: true,
            }
        }

        fn batch_lengths(&self) -> Vec<usize> {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .map(Vec::len)
                .collect()
        }

        fn flattened_inputs(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .flat_map(|batch| batch.iter().cloned())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.lock().expect("calls lock").push(texts.to_vec());
            if self.fail {
                return Err(VektorError::Embedding("warm-up failed".to_string()));
            }
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }

        fn dim(&self) -> usize {
            4
        }

        fn name(&self) -> &str {
            "recording"
        }

        fn prefix_for_document(&self) -> &str {
            "search_document: "
        }

        fn prefix_for_query(&self) -> &str {
            "search_query: "
        }
    }

    #[tokio::test]
    async fn warmup_runs_batch_one_and_batch_thirty_two_with_document_prefix() {
        let embedder = RecordingEmbedder::new();

        WarmUp::run(&embedder).await.expect("warm-up succeeds");

        assert_eq!(embedder.batch_lengths(), [1, 32]);
        assert!(
            embedder
                .flattened_inputs()
                .iter()
                .all(|input| input.starts_with("search_document: ")),
            "warm-up must use document embedding path"
        );
    }

    #[tokio::test]
    async fn warmup_returns_embedding_errors() {
        let embedder = RecordingEmbedder::failing();

        let error = WarmUp::run(&embedder)
            .await
            .expect_err("warm-up failure must propagate");

        assert!(matches!(error, VektorError::Embedding(_)));
        assert_eq!(embedder.batch_lengths(), [1]);
    }
}
