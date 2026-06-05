use std::future;

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
};

use crate::{
    config::Config,
    embedder::Embedder,
    error::VektorError,
    index_status::IndexStatusTracker,
    mcp::handlers::ContextQueryCache,
    mcp::handlers::{self, EmbedderCache, IndexHealthCache},
    warmup::WarmUp,
};

pub async fn start_stdio_server(config: Config) -> crate::error::Result<()> {
    let embedder_cache = EmbedderCache::default();
    warm_server_embedder(&config, &embedder_cache).await?;

    let service = VektorServer {
        config,
        embedder_cache,
        health_cache: IndexHealthCache::default(),
        context_cache: ContextQueryCache::default(),
        status_tracker: IndexStatusTracker::default(),
    }
    .serve(stdio())
    .await
    .map_err(|error| VektorError::Mcp(error.to_string()))?;

    service
        .waiting()
        .await
        .map_err(|error| VektorError::Mcp(error.to_string()))?;

    Ok(())
}

async fn warm_server_embedder(config: &Config, cache: &EmbedderCache) -> crate::error::Result<()> {
    if warmup_skipped_by_env() {
        tracing::warn!("server embedder warm-up skipped by explicit environment override");
        return Ok(());
    }
    let embedder = cache
        .get(config)
        .await
        .map_err(|error| VektorError::Embedding(format!("server warm-up failed: {error}")))?;
    run_startup_warmup(embedder.as_ref()).await
}

async fn run_startup_warmup(embedder: &dyn Embedder) -> crate::error::Result<()> {
    WarmUp::run(embedder).await
}

fn warmup_skipped_by_env() -> bool {
    std::env::var_os("VEKTOR_UNSAFE_SKIP_WARMUP").is_some()
}

#[derive(Clone)]
struct VektorServer {
    config: Config,
    embedder_cache: EmbedderCache,
    health_cache: IndexHealthCache,
    context_cache: ContextQueryCache,
    status_tracker: IndexStatusTracker,
}

impl ServerHandler for VektorServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::default())
            .with_server_info(Implementation::new("vektor", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Vektor exposes tools for indexing and searching local codebases over MCP.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        future::ready(Ok(ListToolsResult::with_all_items(
            crate::mcp::schemas::tools(),
        )))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        crate::mcp::schemas::get_tool(name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // Handlers never write to stdout (the stdio transport owns it); tool
        // errors surface as JSON in the structured result rather than as
        // protocol errors.
        let result: Result<serde_json::Value, ErrorData> = match request.name.as_ref() {
            "index_codebase" => Ok(handlers::handle_index_codebase_with_state(
                request.arguments,
                self.config.clone(),
                self.health_cache.clone(),
                self.context_cache.clone(),
                self.status_tracker.clone(),
            )
            .await),
            "search_code" => Ok(handlers::handle_search_code_with_state(
                request.arguments,
                self.config.clone(),
                self.embedder_cache.clone(),
                self.health_cache.clone(),
                self.status_tracker.clone(),
            )
            .await),
            "get_context_for_prompt" => Ok(handlers::handle_get_context_for_prompt_with_state(
                request.arguments,
                self.config.clone(),
                self.embedder_cache.clone(),
                self.health_cache.clone(),
                self.context_cache.clone(),
                self.status_tracker.clone(),
            )
            .await),
            other => Err(ErrorData::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        };

        result.map(CallToolResult::structured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result as VektorResult;
    use std::sync::Mutex;

    struct RecordingEmbedder {
        calls: Mutex<Vec<usize>>,
    }

    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed(&self, texts: &[String]) -> VektorResult<Vec<Vec<f32>>> {
            self.calls.lock().expect("calls lock").push(texts.len());
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }

        fn dim(&self) -> usize {
            4
        }

        fn name(&self) -> &str {
            "server-recording"
        }

        fn prefix_for_document(&self) -> &str {
            ""
        }

        fn prefix_for_query(&self) -> &str {
            ""
        }
    }

    #[tokio::test]
    async fn server_startup_warmup_runs_before_serving() {
        let embedder = RecordingEmbedder {
            calls: Mutex::new(Vec::new()),
        };

        run_startup_warmup(&embedder)
            .await
            .expect("server warm-up succeeds");

        assert_eq!(*embedder.calls.lock().expect("calls lock"), [1, 32]);
    }
}
