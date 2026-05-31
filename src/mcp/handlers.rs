use std::future::Future;
use std::path::Path;

use rmcp::model::JsonObject;
use serde_json::{Value, json};

use crate::cli::IndexStats;
use crate::config::Config;

/// Build or refresh the local codebase index (Phase 3: vector-only).
///
/// Async because it builds the embedder + LanceDB store and runs the shared
/// index core. Reads the required `path` argument and the optional `force_full`
/// flag from the MCP tool arguments, loads the default [`Config`], and calls the
/// SAME [`crate::cli::index_path`] core the `vektor index` CLI uses — there is no
/// duplicated indexing loop.
///
/// On success it returns real stats JSON
/// (`{ status, files, changed, unchanged, failed, chunks, embeddings, reused,
/// skipped_secrets }`). On any error it returns a JSON error object
/// (`{ status: "error", error: "<message>" }`) rather than panicking.
///
/// It NEVER writes to stdout: stdout is the stdio MCP transport channel, so all
/// diagnostics go through `tracing`.
///
/// Phase 4 (task 4.6) extends this same handler to also populate the Tantivy
/// BM25 index — it does not rebuild this path.
pub async fn handle_index_codebase(args: Option<JsonObject>) -> Value {
    // Production indexer: load default config + run the shared CLI index core.
    let result = run_index_tool(args, |path, force| async move {
        let config = Config::load(None).map_err(|e| e.to_string())?;
        crate::cli::index_path(Path::new(&path), &config, force)
            .await
            .map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(value) => value,
        Err(message) => {
            tracing::error!(error = %message, "index_codebase failed");
            json!({ "status": "error", "error": message })
        }
    }
}

/// Generic core of the `index_codebase` tool: parse args, run `indexer`, map the
/// resulting [`IndexStats`] to JSON.
///
/// `indexer` is injected so tests can run the full tool flow (arg parsing →
/// index → stats JSON) with a FAKE embedder instead of the production
/// `index_path` (which needs a downloaded model). The production handler passes
/// the real `index_path` core.
async fn run_index_tool<F, Fut>(args: Option<JsonObject>, indexer: F) -> Result<Value, String>
where
    F: FnOnce(String, bool) -> Fut,
    Fut: Future<Output = Result<IndexStats, String>>,
{
    let args = args.ok_or_else(|| "missing arguments: `path` is required".to_string())?;

    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `path` argument".to_string())?
        .to_string();

    reject_unsupported_arg(&args, "extensions")?;
    reject_unsupported_arg(&args, "embedding_backend")?;

    // MCP advertises `force_full`; the CLI calls the same flag `force`.
    let force = args
        .get("force_full")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    tracing::info!(path, force, "index_codebase requested via MCP");

    let stats = indexer(path, force).await?;
    Ok(stats_to_json(&stats))
}

fn reject_unsupported_arg(args: &JsonObject, name: &str) -> Result<(), String> {
    if args.contains_key(name) {
        return Err(format!(
            "`{name}` is advertised by the index_codebase schema but is not supported by the Phase 3 vector-only handler yet"
        ));
    }
    Ok(())
}

/// Map run-level [`IndexStats`] to the tool's success JSON response.
fn stats_to_json(stats: &IndexStats) -> Value {
    json!({
        "status": "indexed",
        "files": stats.files,
        "changed": stats.changed,
        "unchanged": stats.unchanged,
        "failed": stats.failed,
        "chunks": stats.chunks,
        "embeddings": stats.embeddings,
        "reused": stats.reused,
        "skipped_secrets": stats.skipped_secrets,
    })
}

pub fn handle_search_code(_args: Option<rmcp::model::JsonObject>) -> Value {
    json!({
        "status": "not implemented yet",
        "phase": "Phase 4 — Search + MCP",
        "see": "docs/plans/initial/phase-4-search-mcp/README.md"
    })
}

pub fn handle_get_context_for_prompt(_args: Option<rmcp::model::JsonObject>) -> Value {
    json!({
        "status": "not implemented yet",
        "phase": "Phase 5 — Context Assembly (v0.4.0)",
        "see": "docs/plans/initial/phase-5-context-assembly/README.md"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::config::Config;
    use crate::embedder::Embedder;
    use crate::error::Result as VektorResult;
    use crate::vector_store::VectorStore;

    const TEST_DIM: usize = 8;
    const TEST_MODEL: &str = "fake-embedder";

    /// Fake embedder: deterministic vectors, no model download.
    struct FakeEmbedder {
        texts: AtomicUsize,
    }

    impl FakeEmbedder {
        fn new() -> Self {
            Self {
                texts: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> VektorResult<Vec<Vec<f32>>> {
            self.texts.fetch_add(texts.len(), Ordering::SeqCst);
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32; TEST_DIM])
                .collect())
        }
        fn dim(&self) -> usize {
            TEST_DIM
        }
        fn name(&self) -> &str {
            TEST_MODEL
        }
        fn prefix_for_document(&self) -> &str {
            ""
        }
        fn prefix_for_query(&self) -> &str {
            ""
        }
    }

    #[test]
    fn skeleton_handlers_return_status_and_phase() {
        for response in [
            handle_search_code(None),
            handle_get_context_for_prompt(None),
        ] {
            assert_eq!(response["status"], "not implemented yet");
            assert!(response["phase"].is_string());
        }
    }

    /// `handle_index_codebase` with no args returns a JSON error object, never panics.
    #[tokio::test]
    async fn index_codebase_missing_args_returns_json_error() {
        let response = handle_index_codebase(None).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("path"),
            "error must mention the required path argument: {response}"
        );
    }

    #[tokio::test]
    async fn index_codebase_rejects_extensions_until_supported() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(tempdir.path().to_string_lossy().into_owned()),
        );
        args.insert(
            "extensions".into(),
            Value::Array(vec![Value::String("rs".into())]),
        );

        let response = handle_index_codebase(Some(args)).await;

        assert_eq!(response["status"], "error");
        let error = response["error"].as_str().expect("error string");
        assert!(error.contains("extensions"), "{response}");
        assert!(error.contains("not supported"), "{response}");
    }

    #[tokio::test]
    async fn index_codebase_rejects_embedding_backend_until_supported() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(tempdir.path().to_string_lossy().into_owned()),
        );
        args.insert("embedding_backend".into(), Value::String("openai".into()));

        let response = handle_index_codebase(Some(args)).await;

        assert_eq!(response["status"], "error");
        let error = response["error"].as_str().expect("error string");
        assert!(error.contains("embedding_backend"), "{response}");
        assert!(error.contains("not supported"), "{response}");
    }

    /// End-to-end through the tool flow with a FAKE embedder seam: indexes a real
    /// tempdir, returns real stats JSON, and (by construction) writes nothing to
    /// stdout — the handler never calls `println!`/`print!`, only returns JSON and
    /// logs via tracing.
    #[tokio::test]
    async fn index_codebase_with_fake_embedder_returns_real_stats_json() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(repo.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write");
        // A secret-bearing chunk to exercise skipped_secrets in the response.
        std::fs::write(
            repo.join("notes.txt"),
            "notes\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
        )
        .expect("write notes");

        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );

        // Inject the fake embedder by running the shared `index_path_with_embedder`
        // seam instead of the production `index_path` (which needs a model).
        let config_for_closure = config.clone();
        let response = run_index_tool(Some(args), |path, force| async move {
            let (root, files) =
                crate::cli::collect_index_files(Path::new(&path), &config_for_closure)
                    .map_err(|e| e.to_string())?;
            let embedder = FakeEmbedder::new();
            let store = VectorStore::new(&root, &config_for_closure, TEST_DIM, TEST_MODEL)
                .await
                .map_err(|e| e.to_string())?;
            crate::cli::index_path_with_embedder(
                &root,
                files,
                &config_for_closure,
                &embedder,
                store,
                force,
            )
            .await
            .map_err(|e| e.to_string())
        })
        .await
        .expect("tool should succeed");

        assert_eq!(response["status"], "indexed");
        assert!(
            response["files"].as_u64().expect("files") >= 2,
            "both files counted: {response}"
        );
        assert!(
            response["chunks"].as_u64().expect("chunks") >= 1,
            "lib.rs produced chunks: {response}"
        );
        assert!(
            response["embeddings"].as_u64().expect("embeddings") >= 1,
            "fresh chunks embedded: {response}"
        );
        assert!(
            response["skipped_secrets"]
                .as_u64()
                .expect("skipped_secrets")
                >= 1,
            "the AWS secret chunk is skipped + counted: {response}"
        );
    }

    /// Bad path returns a JSON error object (no panic). Errors at path validation
    /// before building the embedder, so no model is needed.
    #[tokio::test]
    async fn index_codebase_bad_path_returns_json_error() {
        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String("/nonexistent/path/for/vektor/test".into()),
        );

        let response = handle_index_codebase(Some(args)).await;
        assert_eq!(response["status"], "error");
        assert!(response["error"].is_string());
    }
}
