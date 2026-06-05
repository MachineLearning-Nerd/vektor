use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use rmcp::model::JsonObject;
use serde_json::{Value, json};

use crate::{
    cli::{IndexOptions, IndexStats},
    config::Config,
    context::{
        assembler::{ContextAssembler, RelatedExpansion},
        cache::QueryCache,
        types::{AssemblyConfig, ChunkSource, Confidence, ContextChunk, ContextPackage, GapReason},
    },
    embedder::{Embedder, build_embedder},
    index_status::{IndexPhase, IndexStatusTracker, project_key},
    search::hybrid::{
        HybridResult, HybridSearchConfig, SearchMode, search_hybrid, search_keyword_only,
        search_semantic_only,
    },
    secrets::SecretDetector,
    shallow_indexer::ShallowIndexer,
    state::{FileStatus, HashStore, hash_file},
    text_index::{KeywordHit, TextIndex},
    vector_store::VectorStore,
};

const FILTERED_SEARCH_CANDIDATE_LIMIT: usize = 10_000;
const MIN_FILTERED_SEARCH_CANDIDATES: usize = 1_024;
const INDEX_HEALTH_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub(crate) struct EmbedderCache {
    inner: Arc<tokio::sync::Mutex<HashMap<EmbedderCacheKey, Arc<dyn Embedder>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EmbedderCacheKey {
    backend: String,
    data_dir: String,
    onnx_model: String,
    openai_base_url: String,
    openai_model: String,
    openai_api_key: String,
    fallback_to_onnx: bool,
    max_requests_per_minute: u32,
}

impl EmbedderCacheKey {
    fn from_config(config: &Config) -> Self {
        Self {
            backend: config.embedding.backend.clone(),
            data_dir: config.index.data_dir.clone(),
            onnx_model: config.embedding.onnx_model.clone(),
            openai_base_url: config.embedding.openai_base_url.clone(),
            openai_model: config.embedding.openai_model.clone(),
            openai_api_key: config.embedding.openai_api_key.clone(),
            fallback_to_onnx: config.embedding.fallback_to_onnx,
            max_requests_per_minute: config.embedding.max_requests_per_minute,
        }
    }
}

impl EmbedderCache {
    pub(crate) async fn get(&self, config: &Config) -> Result<Arc<dyn Embedder>, String> {
        let key = EmbedderCacheKey::from_config(config);
        if let Some(embedder) = self.inner.lock().await.get(&key).cloned() {
            return Ok(embedder);
        }

        let embedder: Arc<dyn Embedder> =
            Arc::from(build_embedder(config).map_err(|e| e.to_string())?);
        let mut cache = self.inner.lock().await;
        Ok(cache.entry(key).or_insert_with(|| embedder.clone()).clone())
    }
}

#[derive(Clone, Default)]
pub(crate) struct IndexHealthCache {
    inner: Arc<tokio::sync::Mutex<HashMap<IndexHealthCacheKey, CachedIndexHealth>>>,
}

#[derive(Clone, Default)]
pub(crate) struct ContextQueryCache {
    inner: Arc<tokio::sync::Mutex<QueryCache>>,
}

impl ContextQueryCache {
    async fn clear(&self) {
        self.inner.lock().await.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IndexHealthCacheKey {
    root: PathBuf,
    data_dir: String,
    max_file_size_kb: u64,
}

impl IndexHealthCacheKey {
    fn from_config(root: &Path, config: &Config) -> Self {
        Self {
            root: root.to_path_buf(),
            data_dir: config.index.data_dir.clone(),
            max_file_size_kb: config.index.max_file_size_kb,
        }
    }
}

#[derive(Debug, Clone)]
struct CachedIndexHealth {
    computed_at: Instant,
    health: IndexHealth,
}

impl IndexHealthCache {
    async fn get(&self, root: &Path, config: &Config) -> IndexHealth {
        let key = IndexHealthCacheKey::from_config(root, config);
        if let Some(cached) = self.inner.lock().await.get(&key).cloned()
            && cached.computed_at.elapsed() < INDEX_HEALTH_CACHE_TTL
        {
            return cached.health;
        }

        let health = index_health(root, config);
        self.inner.lock().await.insert(
            key,
            CachedIndexHealth {
                computed_at: Instant::now(),
                health: health.clone(),
            },
        );
        health
    }

    pub(crate) async fn clear(&self) {
        self.inner.lock().await.clear();
    }
}

/// Build or refresh the local codebase index.
///
/// Async because it builds the embedder + LanceDB/Tantivy stores and runs the
/// shared index core. Reads the required `path` argument and optional
/// `force_full`, `extensions`, and `embedding_backend` arguments from the MCP
/// tool arguments, uses the served [`Config`], and calls the SAME
/// [`crate::cli::index_path_with_options`] core the `vektor index` CLI uses —
/// there is no duplicated indexing loop.
///
/// On success it returns real stats JSON
/// (`{ status, files, changed, unchanged, failed, chunks, embeddings, reused,
/// skipped_secrets }`). On any error it returns a JSON error object
/// (`{ status: "error", error: "<message>" }`) rather than panicking.
///
/// It NEVER writes to stdout: stdout is the stdio MCP transport channel, so all
/// diagnostics go through `tracing`.
#[allow(dead_code)]
pub async fn handle_index_codebase(args: Option<JsonObject>) -> Value {
    let config = match Config::load(None) {
        Ok(config) => config,
        Err(error) => {
            let message = error.to_string();
            tracing::error!(error = %message, "index_codebase failed");
            return json!({ "status": "error", "error": message });
        }
    };
    handle_index_codebase_with_config(args, config).await
}

pub async fn handle_index_codebase_with_config(args: Option<JsonObject>, config: Config) -> Value {
    handle_index_codebase_with_optional_cache(
        args,
        config,
        None,
        None,
        IndexStatusTracker::default(),
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn handle_index_codebase_with_caches(
    args: Option<JsonObject>,
    config: Config,
    health_cache: IndexHealthCache,
    context_cache: ContextQueryCache,
) -> Value {
    handle_index_codebase_with_state(
        args,
        config,
        health_cache,
        context_cache,
        IndexStatusTracker::default(),
    )
    .await
}

pub(crate) async fn handle_index_codebase_with_state(
    args: Option<JsonObject>,
    config: Config,
    health_cache: IndexHealthCache,
    context_cache: ContextQueryCache,
    status_tracker: IndexStatusTracker,
) -> Value {
    handle_index_codebase_with_optional_cache(
        args,
        config,
        Some(&health_cache),
        Some(&context_cache),
        status_tracker,
    )
    .await
}

async fn handle_index_codebase_with_optional_cache(
    args: Option<JsonObject>,
    config: Config,
    health_cache: Option<&IndexHealthCache>,
    context_cache: Option<&ContextQueryCache>,
    status_tracker: IndexStatusTracker,
) -> Value {
    // Production indexer: use the served config + run the shared CLI index core.
    let result = run_index_tool_with_cache_invalidation(
        args,
        |request| async move {
            let mut config = config;
            if let Some(backend) = request.embedding_backend {
                config.embedding.backend = backend;
            }

            let options = IndexOptions {
                force: request.force,
                extensions: request.extensions,
            };
            let root = canonical_project_root(&request.path)?;
            let key = project_key(&root);
            status_tracker.mark_building(&key);

            if options.extensions.is_none() {
                ShallowIndexer::build(Path::new(&request.path), &config)
                    .map_err(|e| e.to_string())?;
                status_tracker.mark_partial(&key);
            }

            let stats =
                crate::cli::index_path_with_options(Path::new(&request.path), &config, options)
                    .await
                    .map_err(|e| e.to_string())?;
            let text_ready = TextIndex::open_readonly(&root, &config)
                .map(|index| index.is_ready())
                .unwrap_or(false);
            if text_ready && stats.failed == 0 {
                status_tracker.mark_full(&key);
            } else {
                status_tracker.mark_partial(&key);
            }
            Ok(stats)
        },
        health_cache,
        context_cache,
    )
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
/// `index_path_with_options` (which needs a downloaded model). The production
/// handler passes the real shared core.
async fn run_index_tool<F, Fut>(args: Option<JsonObject>, indexer: F) -> Result<Value, String>
where
    F: FnOnce(IndexToolRequest) -> Fut,
    Fut: Future<Output = Result<IndexStats, String>>,
{
    let args = args.ok_or_else(|| "missing arguments: `path` is required".to_string())?;

    let path = parse_path_arg(&args)?;

    // MCP advertises `force_full`; the CLI calls the same flag `force`.
    let force = args
        .get("force_full")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extensions = parse_extension_array(&args, "extensions")?;
    let embedding_backend = parse_embedding_backend(&args)?;

    tracing::info!(
        path,
        force,
        extensions = ?extensions,
        embedding_backend = ?embedding_backend,
        "index_codebase requested via MCP"
    );

    let stats = indexer(IndexToolRequest {
        path,
        force,
        extensions,
        embedding_backend,
    })
    .await?;
    Ok(stats_to_json(&stats))
}

async fn run_index_tool_with_cache_invalidation<F, Fut>(
    args: Option<JsonObject>,
    indexer: F,
    health_cache: Option<&IndexHealthCache>,
    context_cache: Option<&ContextQueryCache>,
) -> Result<Value, String>
where
    F: FnOnce(IndexToolRequest) -> Fut,
    Fut: Future<Output = Result<IndexStats, String>>,
{
    let value = run_index_tool(args, indexer).await?;
    if let Some(cache) = health_cache {
        cache.clear().await;
    }
    if let Some(cache) = context_cache {
        cache.clear().await;
    }
    Ok(value)
}

#[derive(Debug)]
struct IndexToolRequest {
    path: String,
    force: bool,
    extensions: Option<Vec<String>>,
    embedding_backend: Option<String>,
}

fn parse_extension_array(args: &JsonObject, name: &str) -> Result<Option<Vec<String>>, String> {
    let Some(value) = args.get(name) else {
        return Ok(None);
    };
    let Value::Array(values) = value else {
        return Err(format!("`{name}` must be an array of strings"));
    };

    let mut extensions = Vec::with_capacity(values.len());
    for value in values {
        let Value::String(raw) = value else {
            return Err(format!("`{name}` entries must be strings"));
        };
        let normalized = raw.trim().trim_start_matches('.').to_ascii_lowercase();
        if normalized.is_empty() {
            return Err(format!("`{name}` entries must not be empty"));
        }
        extensions.push(normalized);
    }

    Ok(Some(extensions))
}

fn parse_embedding_backend(args: &JsonObject) -> Result<Option<String>, String> {
    let Some(value) = args.get("embedding_backend") else {
        return Ok(None);
    };
    let Value::String(raw) = value else {
        return Err("`embedding_backend` must be a string".to_string());
    };
    let backend = raw.trim().to_ascii_lowercase();
    if backend.is_empty() {
        return Err("`embedding_backend` must not be empty".to_string());
    }
    if !matches!(backend.as_str(), "onnx" | "openai") {
        return Err(format!(
            "unsupported `embedding_backend`: \"{raw}\"; supported values are \"onnx\" and \"openai\""
        ));
    }

    Ok(Some(backend))
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

#[allow(dead_code)]
pub async fn handle_search_code(args: Option<JsonObject>) -> Value {
    let config = match Config::load(None) {
        Ok(config) => config,
        Err(error) => {
            let message = error.to_string();
            tracing::error!(error = %message, "search_code failed");
            return json!({ "status": "error", "error": message });
        }
    };
    handle_search_code_with_config(args, config).await
}

pub async fn handle_search_code_with_config(args: Option<JsonObject>, config: Config) -> Value {
    handle_search_code_with_caches(
        args,
        config,
        EmbedderCache::default(),
        IndexHealthCache::default(),
    )
    .await
}

pub(crate) async fn handle_search_code_with_caches(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: EmbedderCache,
    health_cache: IndexHealthCache,
) -> Value {
    handle_search_code_with_state(
        args,
        config,
        embedder_cache,
        health_cache,
        IndexStatusTracker::default(),
    )
    .await
}

pub(crate) async fn handle_search_code_with_state(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: EmbedderCache,
    health_cache: IndexHealthCache,
    status_tracker: IndexStatusTracker,
) -> Value {
    let result = run_search_tool_with_status(
        args,
        config,
        &embedder_cache,
        &health_cache,
        Some(&status_tracker),
    )
    .await;

    match result {
        Ok(value) => value,
        Err(message) => {
            tracing::error!(error = %message, "search_code failed");
            json!({ "status": "error", "error": message })
        }
    }
}

#[allow(dead_code)]
pub async fn handle_get_context_for_prompt(args: Option<JsonObject>) -> Value {
    let config = match Config::load(None) {
        Ok(config) => config,
        Err(error) => {
            let message = error.to_string();
            tracing::error!(error = %message, "get_context_for_prompt failed");
            return json!({ "status": "error", "error": message });
        }
    };
    handle_get_context_for_prompt_with_config(args, config).await
}

pub async fn handle_get_context_for_prompt_with_config(
    args: Option<JsonObject>,
    config: Config,
) -> Value {
    handle_get_context_for_prompt_with_caches(
        args,
        config,
        EmbedderCache::default(),
        IndexHealthCache::default(),
        ContextQueryCache::default(),
    )
    .await
}

pub(crate) async fn handle_get_context_for_prompt_with_caches(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: EmbedderCache,
    health_cache: IndexHealthCache,
    context_cache: ContextQueryCache,
) -> Value {
    handle_get_context_for_prompt_with_state(
        args,
        config,
        embedder_cache,
        health_cache,
        context_cache,
        IndexStatusTracker::default(),
    )
    .await
}

pub(crate) async fn handle_get_context_for_prompt_with_state(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: EmbedderCache,
    health_cache: IndexHealthCache,
    context_cache: ContextQueryCache,
    status_tracker: IndexStatusTracker,
) -> Value {
    let result = run_context_tool_with_status(
        args,
        config,
        &embedder_cache,
        &health_cache,
        &context_cache,
        Some(&status_tracker),
    )
    .await;

    match result {
        Ok(value) => value,
        Err(message) => {
            tracing::error!(error = %message, "get_context_for_prompt failed");
            json!({ "status": "error", "error": message })
        }
    }
}

#[allow(dead_code)]
async fn run_search_tool(args: Option<JsonObject>) -> Result<Value, String> {
    let config = Config::load(None).map_err(|e| e.to_string())?;
    run_search_tool_with_config(
        args,
        config,
        &EmbedderCache::default(),
        &IndexHealthCache::default(),
    )
    .await
}

async fn run_search_tool_with_config(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
) -> Result<Value, String> {
    run_search_tool_with_status(args, config, embedder_cache, health_cache, None).await
}

async fn run_search_tool_with_status(
    args: Option<JsonObject>,
    mut config: Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
    status_tracker: Option<&IndexStatusTracker>,
) -> Result<Value, String> {
    let request = parse_search_tool_request(args)?;
    let root = canonical_project_root(&request.path)?;

    run_search_request_with_status(
        &request,
        &root,
        &mut config,
        embedder_cache,
        health_cache,
        status_tracker,
    )
    .await
}

#[allow(dead_code)]
async fn run_search_request(
    request: &SearchToolRequest,
    root: &Path,
    config: &mut Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
) -> Result<Value, String> {
    run_search_request_with_status(request, root, config, embedder_cache, health_cache, None).await
}

async fn run_search_request_with_status(
    request: &SearchToolRequest,
    root: &Path,
    config: &mut Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
    status_tracker: Option<&IndexStatusTracker>,
) -> Result<Value, String> {
    if let Some(tracker) = status_tracker {
        match tracker.status_for_root(root, config) {
            IndexPhase::Building => {
                return Ok(search_results_to_json(
                    &[],
                    request.mode,
                    0,
                    IndexPhase::Building.as_str(),
                    0.0,
                    vec!["index is building; results are unavailable until shallow indexing completes".to_string()],
                ));
            }
            IndexPhase::Partial => {
                return run_search_text_index_only_cached(
                    request,
                    root,
                    config,
                    Some(health_cache),
                    IndexPhase::Partial,
                )
                .await;
            }
            IndexPhase::Full => {}
        }
    }

    apply_indexed_backend(root, config)?;
    if request.mode == SearchMode::Keyword {
        return run_search_keyword_only_cached(request, root, config, Some(health_cache)).await;
    }

    ensure_vector_index_exists(root, config)?;
    let embedder = embedder_cache.get(config).await?;

    run_search_with_embedder_cached(request, root, config, embedder.as_ref(), Some(health_cache))
        .await
}

#[allow(dead_code)]
async fn run_search_keyword_only(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
) -> Result<Value, String> {
    run_search_keyword_only_cached(request, root, config, None).await
}

async fn run_search_keyword_only_cached(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
    health_cache: Option<&IndexHealthCache>,
) -> Result<Value, String> {
    tracing::info!(
        path = %root.display(),
        query = %request.query,
        top_k = request.top_k,
        mode = ?request.mode,
        "search_code requested via MCP"
    );

    let filter = extension_filter_predicate(request.filter_ext.as_deref());
    let search_limit = expanded_search_limit(request.top_k, request.filter_ext.is_some());
    let (results, search_time_ms) =
        search_project_keyword_only(&request.query, search_limit, filter, root, config)
            .await
            .map_err(|e| e.to_string())?;
    let results = filter_results(
        results,
        &ResultFilters {
            filter_ext: request.filter_ext.as_deref(),
            min_relevance: None,
            include_docs: true,
            scope: None,
            max_files: None,
            limit: Some(request.top_k),
        },
    );
    let IndexHealth {
        status,
        coverage_pct,
        mut warnings,
    } = index_health_for(root, config, health_cache).await;
    if request.bypass_cache {
        warnings.push(
            "`bypass_cache` was accepted, but Phase 4 does not implement query caching".to_string(),
        );
    }

    Ok(search_results_to_json(
        &results,
        request.mode,
        search_time_ms,
        &status,
        coverage_pct,
        warnings,
    ))
}

async fn run_search_text_index_only_cached(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
    health_cache: Option<&IndexHealthCache>,
    phase: IndexPhase,
) -> Result<Value, String> {
    tracing::info!(
        path = %root.display(),
        query = %request.query,
        top_k = request.top_k,
        mode = ?request.mode,
        index_status = phase.as_str(),
        "search_code requested via MCP using shallow keyword tier"
    );

    let search_limit = expanded_search_limit(request.top_k, request.filter_ext.is_some());
    let (results, search_time_ms) =
        search_project_text_index_only(&request.query, search_limit, root, config)
            .map_err(|e| e.to_string())?;
    let results = filter_results(
        results,
        &ResultFilters {
            filter_ext: request.filter_ext.as_deref(),
            min_relevance: None,
            include_docs: true,
            scope: None,
            max_files: None,
            limit: Some(request.top_k),
        },
    );
    let health = index_health_for_phase(root, config, health_cache, phase).await;
    Ok(search_results_to_json(
        &results,
        SearchMode::Keyword,
        search_time_ms,
        &health.status,
        health.coverage_pct,
        health.warnings,
    ))
}

#[allow(dead_code)]
async fn run_search_with_embedder(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
) -> Result<Value, String> {
    run_search_with_embedder_cached(request, root, config, embedder, None).await
}

async fn run_search_with_embedder_cached(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
    health_cache: Option<&IndexHealthCache>,
) -> Result<Value, String> {
    tracing::info!(
        path = %root.display(),
        query = %request.query,
        top_k = request.top_k,
        mode = ?request.mode,
        "search_code requested via MCP"
    );

    let filter = extension_filter_predicate(request.filter_ext.as_deref());
    let search_limit = expanded_search_limit(request.top_k, request.filter_ext.is_some());
    let (results, search_time_ms) = search_project(
        &request.query,
        request.mode,
        search_limit,
        filter,
        root,
        config,
        embedder,
    )
    .await
    .map_err(|e| e.to_string())?;
    let results = filter_results(
        results,
        &ResultFilters {
            filter_ext: request.filter_ext.as_deref(),
            min_relevance: None,
            include_docs: true,
            scope: None,
            max_files: None,
            limit: Some(request.top_k),
        },
    );
    let IndexHealth {
        status,
        coverage_pct,
        mut warnings,
    } = index_health_for(root, config, health_cache).await;
    if request.bypass_cache {
        warnings.push(
            "`bypass_cache` was accepted, but Phase 4 does not implement query caching".to_string(),
        );
    }

    Ok(search_results_to_json(
        &results,
        request.mode,
        search_time_ms,
        &status,
        coverage_pct,
        warnings,
    ))
}

#[allow(dead_code)]
async fn run_context_tool(args: Option<JsonObject>) -> Result<Value, String> {
    let config = Config::load(None).map_err(|e| e.to_string())?;
    run_context_tool_with_config(
        args,
        config,
        &EmbedderCache::default(),
        &IndexHealthCache::default(),
        &ContextQueryCache::default(),
    )
    .await
}

async fn run_context_tool_with_config(
    args: Option<JsonObject>,
    config: Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
    context_cache: &ContextQueryCache,
) -> Result<Value, String> {
    run_context_tool_with_status(
        args,
        config,
        embedder_cache,
        health_cache,
        context_cache,
        None,
    )
    .await
}

async fn run_context_tool_with_status(
    args: Option<JsonObject>,
    mut config: Config,
    embedder_cache: &EmbedderCache,
    health_cache: &IndexHealthCache,
    context_cache: &ContextQueryCache,
    status_tracker: Option<&IndexStatusTracker>,
) -> Result<Value, String> {
    let request = parse_context_tool_request(args)?;
    let root = canonical_project_root(&request.path)?;

    if let Some(tracker) = status_tracker {
        match tracker.status_for_root(&root, &config) {
            IndexPhase::Building => {
                return assemble_context_from_results(
                    &request,
                    &root,
                    &config,
                    ContextAssemblyRun {
                        results: Vec::new(),
                        search_time_ms: 0,
                        cache_hit: false,
                        phase: IndexPhase::Building,
                    },
                    Some(health_cache),
                )
                .await;
            }
            IndexPhase::Partial => {
                let search_limit =
                    expanded_search_limit(request.max_files.saturating_mul(4).max(8), true);
                let project_hash = project_hash(&root);
                let mut cache_hit = false;
                let results = if !request.bypass_cache {
                    cached_context_results(
                        Some(context_cache),
                        &request.query,
                        SearchMode::Keyword,
                        &project_hash,
                    )
                    .await
                } else {
                    None
                };
                let (results, search_time_ms) = if let Some(results) = results {
                    cache_hit = true;
                    (results, 0)
                } else {
                    let (results, elapsed_ms) = search_project_text_index_only(
                        &request.query,
                        search_limit,
                        &root,
                        &config,
                    )
                    .map_err(|e| e.to_string())?;
                    put_context_results(
                        Some(context_cache),
                        &request.query,
                        SearchMode::Keyword,
                        &project_hash,
                        results.clone(),
                    )
                    .await;
                    (results, elapsed_ms)
                };
                return assemble_context_from_results(
                    &request,
                    &root,
                    &config,
                    ContextAssemblyRun {
                        results,
                        search_time_ms,
                        cache_hit,
                        phase: IndexPhase::Partial,
                    },
                    Some(health_cache),
                )
                .await;
            }
            IndexPhase::Full => {}
        }
    }

    apply_indexed_backend(&root, &mut config)?;
    let embedder = embedder_cache.get(&config).await?;

    run_context_with_embedder_cached(
        &request,
        &root,
        &config,
        embedder.as_ref(),
        Some(health_cache),
        Some(context_cache),
    )
    .await
}

struct ContextAssemblyRun {
    results: Vec<HybridResult>,
    search_time_ms: u128,
    cache_hit: bool,
    phase: IndexPhase,
}

async fn assemble_context_from_results(
    request: &ContextToolRequest,
    root: &Path,
    config: &Config,
    run: ContextAssemblyRun,
    health_cache: Option<&IndexHealthCache>,
) -> Result<Value, String> {
    let health = index_health_for_phase(root, config, health_cache, run.phase).await;
    let assembly_config = AssemblyConfig {
        token_budget: request.token_budget,
        max_files: request.max_files,
        include_related: request.include_related,
        min_relevance: request.min_relevance,
        deduplicate: true,
        include_docs: request.include_docs,
        scope: request.scope.clone(),
    };
    let assembler = ContextAssembler::with_status(&health.status).with_metadata(
        run.search_time_ms,
        health.coverage_pct,
        run.cache_hit,
    );
    let package = assembler
        .assemble::<VectorStore>(run.results, &assembly_config, None)
        .await
        .map_err(|e| e.to_string())?;

    Ok(context_package_to_json(&package, health.warnings))
}

#[allow(dead_code)]
async fn run_context_with_embedder(
    request: &ContextToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
) -> Result<Value, String> {
    run_context_with_embedder_cached(request, root, config, embedder, None, None).await
}

async fn run_context_with_embedder_cached(
    request: &ContextToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
    health_cache: Option<&IndexHealthCache>,
    context_cache: Option<&ContextQueryCache>,
) -> Result<Value, String> {
    tracing::info!(
        path = %root.display(),
        query = %request.query,
        max_files = request.max_files,
        min_relevance = request.min_relevance,
        include_docs = request.include_docs,
        scope = ?request.scope,
        "get_context_for_prompt requested via MCP"
    );

    let search_limit = expanded_search_limit(request.max_files.saturating_mul(4).max(8), true);
    let project_hash = project_hash(root);
    let mut cache_hit = false;
    let mut search_time_ms = 0;
    let results = if !request.bypass_cache {
        cached_context_results(
            context_cache,
            &request.query,
            SearchMode::Hybrid,
            &project_hash,
        )
        .await
    } else {
        None
    };
    let results = if let Some(results) = results {
        cache_hit = true;
        results
    } else {
        let (results, elapsed_ms) = search_project(
            &request.query,
            SearchMode::Hybrid,
            search_limit,
            None,
            root,
            config,
            embedder,
        )
        .await
        .map_err(|e| e.to_string())?;
        search_time_ms = elapsed_ms;
        put_context_results(
            context_cache,
            &request.query,
            SearchMode::Hybrid,
            &project_hash,
            results.clone(),
        )
        .await;
        results
    };
    let health = index_health_for(root, config, health_cache).await;
    let assembly_config = AssemblyConfig {
        token_budget: request.token_budget,
        max_files: request.max_files,
        include_related: request.include_related,
        min_relevance: request.min_relevance,
        deduplicate: true,
        include_docs: request.include_docs,
        scope: request.scope.clone(),
    };
    let related_store = if request.include_related {
        Some(
            VectorStore::open_existing_for_model(root, config, embedder.dim(), embedder.name())
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let query_vec = if related_store.is_some() {
        Some(
            embedder
                .embed_query(&request.query)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let assembler = ContextAssembler::with_status(&health.status).with_metadata(
        search_time_ms,
        health.coverage_pct,
        cache_hit,
    );
    let package =
        if let (Some(store), Some(query_vec)) = (related_store.as_ref(), query_vec.as_ref()) {
            assembler
                .assemble(
                    results,
                    &assembly_config,
                    Some(RelatedExpansion { query_vec, store }),
                )
                .await
                .map_err(|e| e.to_string())?
        } else {
            assembler
                .assemble::<VectorStore>(results, &assembly_config, None)
                .await
                .map_err(|e| e.to_string())?
        };

    Ok(context_package_to_json(&package, health.warnings))
}

async fn cached_context_results(
    cache: Option<&ContextQueryCache>,
    query: &str,
    mode: SearchMode,
    project_hash: &str,
) -> Option<Vec<HybridResult>> {
    let cache = cache?;
    cache
        .inner
        .lock()
        .await
        .get(query, mode, project_hash)
        .cloned()
}

async fn put_context_results(
    cache: Option<&ContextQueryCache>,
    query: &str,
    mode: SearchMode,
    project_hash: &str,
    results: Vec<HybridResult>,
) {
    let Some(cache) = cache else {
        return;
    };
    let files_included = results
        .iter()
        .map(|result| result.rel_path.clone())
        .collect::<HashSet<_>>();
    cache
        .inner
        .lock()
        .await
        .put(query, mode, project_hash, results, files_included);
}

fn project_hash(root: &Path) -> String {
    project_key(root)
}

fn search_project_text_index_only(
    query: &str,
    top_k: usize,
    root: &Path,
    config: &Config,
) -> crate::error::Result<(Vec<HybridResult>, u128)> {
    let started_at = Instant::now();
    let text_index = TextIndex::open_readonly(root, config)?;
    let results = text_index
        .search(query, top_k)?
        .into_iter()
        .map(keyword_hit_to_result)
        .collect();

    Ok((results, started_at.elapsed().as_millis()))
}

fn keyword_hit_to_result(hit: KeywordHit) -> HybridResult {
    HybridResult {
        chunk_id: hit.chunk_id,
        rel_path: hit.rel_path,
        start_line: hit.start_line,
        end_line: hit.end_line,
        symbol_name: hit.symbol_name,
        symbol_type: None,
        language: hit.language,
        content: hit.content,
        relevance_score: hit.score,
        semantic_score: None,
        keyword_score: Some(hit.score),
        last_modified: 0,
    }
}

async fn search_project(
    query: &str,
    mode: SearchMode,
    top_k: usize,
    filter: Option<String>,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
) -> crate::error::Result<(Vec<HybridResult>, u128)> {
    let started_at = Instant::now();
    let store =
        VectorStore::open_existing_for_model(root, config, embedder.dim(), embedder.name()).await?;
    let mut search_config = HybridSearchConfig::new(mode, top_k);
    search_config.filter = filter;
    let results = if mode == SearchMode::Semantic {
        search_semantic_only(query, &search_config, &store, embedder).await?
    } else {
        let text_index = TextIndex::open_readonly(root, config)?;
        search_hybrid(query, &search_config, &store, &text_index, embedder).await?
    };

    Ok((results, started_at.elapsed().as_millis()))
}

async fn search_project_keyword_only(
    query: &str,
    top_k: usize,
    filter: Option<String>,
    root: &Path,
    config: &Config,
) -> crate::error::Result<(Vec<HybridResult>, u128)> {
    let started_at = Instant::now();
    let store = VectorStore::open_existing(root, config).await?;
    let text_index = TextIndex::open_readonly(root, config)?;
    let mut search_config = HybridSearchConfig::new(SearchMode::Keyword, top_k);
    search_config.filter = filter;
    let results = search_keyword_only(query, &search_config, &store, &text_index).await?;

    Ok((results, started_at.elapsed().as_millis()))
}

fn apply_indexed_backend(root: &Path, config: &mut Config) -> Result<(), String> {
    let Some(meta) = VectorStore::load_meta(root, config).map_err(|e| e.to_string())? else {
        return Ok(());
    };

    if meta.model_name == config.embedding.openai_model {
        config.embedding.backend = "openai".to_string();
    } else if meta.model_name == config.embedding.onnx_model {
        config.embedding.backend = "onnx".to_string();
    }

    Ok(())
}

fn ensure_vector_index_exists(root: &Path, config: &Config) -> Result<(), String> {
    if VectorStore::load_meta(root, config)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(());
    }

    Err(format!(
        "vector index not found for {}; run `vektor index` first",
        root.display()
    ))
}

#[derive(Debug)]
struct SearchToolRequest {
    path: String,
    query: String,
    top_k: usize,
    mode: SearchMode,
    filter_ext: Option<Vec<String>>,
    bypass_cache: bool,
}

fn parse_search_tool_request(args: Option<JsonObject>) -> Result<SearchToolRequest, String> {
    let args =
        args.ok_or_else(|| "missing arguments: `query` and `path` are required".to_string())?;

    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `query` argument".to_string())?
        .trim()
        .to_string();
    if query.is_empty() {
        return Err("`query` must not be empty".to_string());
    }

    let path = parse_path_arg(&args)?;

    let top_k = match args.get("top_k") {
        Some(value) => {
            let raw = value
                .as_u64()
                .ok_or_else(|| "`top_k` must be a positive integer".to_string())?;
            if raw == 0 {
                return Err("`top_k` must be at least 1".to_string());
            }
            usize::try_from(raw).map_err(|_| "`top_k` is too large".to_string())?
        }
        None => 8,
    };

    let mode = match args.get("mode") {
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| "`mode` must be a string".to_string())?;
            SearchMode::parse(raw).map_err(|e| e.to_string())?
        }
        None => SearchMode::Hybrid,
    };

    let bypass_cache = parse_bool_arg(&args, "bypass_cache", false)?;
    let filter_ext = parse_extension_array(&args, "filter_ext")?;

    Ok(SearchToolRequest {
        path,
        query,
        top_k,
        mode,
        filter_ext,
        bypass_cache,
    })
}

#[derive(Debug)]
struct ContextToolRequest {
    path: String,
    query: String,
    token_budget: usize,
    max_files: usize,
    include_related: bool,
    min_relevance: f32,
    include_docs: bool,
    bypass_cache: bool,
    scope: Option<String>,
}

fn parse_context_tool_request(args: Option<JsonObject>) -> Result<ContextToolRequest, String> {
    let args =
        args.ok_or_else(|| "missing arguments: `query` and `path` are required".to_string())?;

    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `query` argument".to_string())?
        .trim()
        .to_string();
    if query.is_empty() {
        return Err("`query` must not be empty".to_string());
    }

    let path = parse_path_arg(&args)?;

    let token_budget = parse_positive_usize_arg(&args, "token_budget", 8_000)?;
    let max_files = parse_positive_usize_arg(&args, "max_files", 10)?;
    let include_related = parse_bool_arg(&args, "include_related", true)?;
    let min_relevance = parse_min_relevance(&args)?;
    let include_docs = parse_bool_arg(&args, "include_docs", true)?;
    let bypass_cache = parse_bool_arg(&args, "bypass_cache", false)?;
    let scope = parse_scope(&args)?;

    Ok(ContextToolRequest {
        path,
        query,
        token_budget,
        max_files,
        include_related,
        min_relevance,
        include_docs,
        bypass_cache,
        scope,
    })
}

fn parse_positive_usize_arg(
    args: &JsonObject,
    name: &str,
    default: usize,
) -> Result<usize, String> {
    let Some(value) = args.get(name) else {
        return Ok(default);
    };
    let raw = value
        .as_u64()
        .ok_or_else(|| format!("`{name}` must be a positive integer"))?;
    if raw == 0 {
        return Err(format!("`{name}` must be at least 1"));
    }
    usize::try_from(raw).map_err(|_| format!("`{name}` is too large"))
}

fn parse_path_arg(args: &JsonObject) -> Result<String, String> {
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `path` argument".to_string())?;
    let path = raw.trim();
    if path.is_empty() {
        return Err("`path` must not be empty".to_string());
    }
    Ok(path.to_string())
}

fn parse_bool_arg(args: &JsonObject, name: &str, default: bool) -> Result<bool, String> {
    match args.get(name) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("`{name}` must be a boolean")),
        None => Ok(default),
    }
}

fn parse_min_relevance(args: &JsonObject) -> Result<f32, String> {
    let Some(value) = args.get("min_relevance") else {
        return Ok(0.5);
    };
    let raw = value
        .as_f64()
        .ok_or_else(|| "`min_relevance` must be a number".to_string())?;
    if !(0.0..=1.0).contains(&raw) {
        return Err("`min_relevance` must be between 0.0 and 1.0".to_string());
    }
    Ok(raw as f32)
}

fn parse_scope(args: &JsonObject) -> Result<Option<String>, String> {
    let Some(value) = args.get("scope") else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| "`scope` must be a string".to_string())?;
    let scope = normalize_scope(raw);
    if scope.is_empty() {
        return Err("`scope` must not be empty".to_string());
    }
    Ok(Some(scope))
}

fn normalize_scope(scope: &str) -> String {
    scope
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

fn canonical_project_root(path: &str) -> Result<PathBuf, String> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("invalid `path` argument `{path}`: {e}"))?;
    if canonical.is_file() {
        return canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("invalid `path` argument `{path}`: file has no parent"));
    }
    Ok(canonical)
}

fn search_results_to_json(
    results: &[HybridResult],
    mode: SearchMode,
    search_time_ms: u128,
    index_status: &str,
    index_coverage_pct: f64,
    missing_context_warnings: Vec<String>,
) -> Value {
    let max_score = max_relevance_score(results);
    json!({
        "results": results
            .iter()
            .map(|result| search_result_to_json(result, max_score))
            .collect::<Vec<_>>(),
        "metadata": {
            "search_time_ms": search_time_ms,
            "mode": search_mode_name(mode),
            "cache_hit": false,
            "index_status": index_status,
            "index_coverage_pct": index_coverage_pct,
            "result_confidence": result_confidence(results, 0.0),
            "missing_context_warnings": missing_context_warnings,
        }
    })
}

fn search_result_to_json(result: &HybridResult, max_score: f32) -> Value {
    json!({
        "chunk_id": result.chunk_id.as_str(),
        "file": result.rel_path.as_str(),
        "lines": line_range(result),
        "symbol": result.symbol_name.as_deref(),
        "type": result.symbol_type.as_deref(),
        "language": result.language.as_str(),
        "score": normalized_relevance(result, max_score),
        "reason": format!("Top {} match for query", search_mode_name_for_scores(result)),
        "snippet": result.content.as_str(),
    })
}

fn context_package_to_json(package: &ContextPackage, health_warnings: Vec<String>) -> Value {
    let warnings = merged_warnings(health_warnings, package.missing_context_warnings.clone());
    json!({
        "context": package
            .chunks
            .iter()
            .map(context_chunk_to_json)
            .collect::<Vec<_>>(),
        "metadata": {
            "files_included": package.files_included.len(),
            "total_tokens": package.total_tokens,
            "budget_used_pct": package.budget_used_pct,
            "chunks_returned": package.chunks.len(),
            "chunks_deduplicated": package.chunks_deduplicated,
            "search_time_ms": package.search_metadata.search_time_ms,
            "cache_hit": package.cache_hit,
            "index_status": package.index_status,
            "index_coverage_pct": package.index_coverage_pct,
            "result_confidence": confidence_name(package.result_confidence),
            "budget_gap_reason": gap_reason_value(package.budget_gap_reason),
            "missing_context_warnings": warnings,
            "suggested_action": package.suggested_action,
            "clusters": package.clusters.iter().map(cluster_to_json).collect::<Vec<_>>(),
        }
    })
}

fn context_chunk_to_json(chunk: &ContextChunk) -> Value {
    json!({
        "file": chunk.rel_path.as_str(),
        "lines": format!("{}-{}", chunk.lines.0, chunk.lines.1),
        "symbol": chunk.symbol.as_deref(),
        "type": chunk.symbol_type.as_deref(),
        "language": chunk.language.as_str(),
        "relevance": chunk.relevance_score.clamp(0.0, 1.0),
        "source": chunk_source_name(chunk.source),
        "reason": chunk.reason.as_str(),
        "content": chunk.content.as_str(),
    })
}

fn merged_warnings(first: Vec<String>, second: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    for warning in first.into_iter().chain(second) {
        if seen.insert(warning.clone()) {
            warnings.push(warning);
        }
    }
    warnings
}

fn chunk_source_name(source: ChunkSource) -> &'static str {
    match source {
        ChunkSource::Search => "search",
        ChunkSource::Related => "related",
        ChunkSource::Dependency => "dependency",
    }
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn gap_reason_value(reason: Option<GapReason>) -> Value {
    match reason {
        Some(GapReason::NoMoreRelevant) => json!("no_more_relevant"),
        Some(GapReason::IndexIncomplete) => json!("index_incomplete"),
        Some(GapReason::ThresholdFiltered) => json!("threshold_filtered"),
        None => Value::Null,
    }
}

fn cluster_to_json(cluster: &crate::context::types::ResultCluster) -> Value {
    json!({
        "path": cluster.path_prefix.as_str(),
        "chunk_count": cluster.chunk_count,
        "avg_relevance": cluster.avg_relevance,
    })
}

fn line_range(result: &HybridResult) -> String {
    format!("{}-{}", result.start_line, result.end_line)
}

fn search_mode_name(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Hybrid => "hybrid",
        SearchMode::Semantic => "semantic",
        SearchMode::Keyword => "keyword",
    }
}

fn search_mode_name_for_scores(result: &HybridResult) -> &'static str {
    match (result.semantic_score, result.keyword_score) {
        (Some(_), Some(_)) => "hybrid",
        (Some(_), None) => "semantic",
        (None, Some(_)) => "keyword",
        (None, None) => "hybrid",
    }
}

fn expanded_search_limit(limit: usize, has_post_filters: bool) -> usize {
    if has_post_filters {
        limit.saturating_mul(64).max(limit).clamp(
            MIN_FILTERED_SEARCH_CANDIDATES,
            FILTERED_SEARCH_CANDIDATE_LIMIT,
        )
    } else {
        limit
    }
}

#[derive(Debug)]
struct ResultFilters<'a> {
    filter_ext: Option<&'a [String]>,
    min_relevance: Option<f32>,
    include_docs: bool,
    scope: Option<&'a str>,
    max_files: Option<usize>,
    limit: Option<usize>,
}

fn filter_results(results: Vec<HybridResult>, filters: &ResultFilters<'_>) -> Vec<HybridResult> {
    let hard_filtered = results
        .into_iter()
        .filter(|result| match filters.filter_ext {
            Some(extensions) => path_matches_extensions(&result.rel_path, extensions),
            None => true,
        })
        .filter(|result| filters.include_docs || !is_obvious_doc_path(&result.rel_path))
        .filter(|result| match filters.scope {
            Some(scope) => path_in_scope(&result.rel_path, scope),
            None => true,
        })
        .collect::<Vec<_>>();
    let max_score = max_relevance_score(&hard_filtered);
    let mut filtered = Vec::new();
    let mut files = HashSet::new();

    for result in hard_filtered {
        if filters
            .min_relevance
            .is_some_and(|min_relevance| normalized_relevance(&result, max_score) < min_relevance)
        {
            continue;
        }
        if let Some(max_files) = filters.max_files
            && !files.contains(&result.rel_path)
            && files.len() >= max_files
        {
            continue;
        }

        files.insert(result.rel_path.clone());
        filtered.push(result);

        if filters.limit.is_some_and(|limit| filtered.len() >= limit) {
            break;
        }
    }

    filtered
}

fn path_matches_extensions(rel_path: &str, extensions: &[String]) -> bool {
    let Some(extension) = Path::new(rel_path).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    extensions.iter().any(|allowed| allowed == &extension)
}

fn is_obvious_doc_path(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    if lower.starts_with("docs/") || lower.contains("/docs/") {
        return true;
    }

    let Some(extension) = Path::new(&lower).extension().and_then(|ext| ext.to_str()) else {
        return lower == "readme" || lower.ends_with("/readme");
    };

    matches!(
        extension,
        "adoc" | "asciidoc" | "md" | "markdown" | "rst" | "txt"
    )
}

fn path_in_scope(rel_path: &str, scope: &str) -> bool {
    rel_path == scope
        || rel_path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn extension_filter_predicate(extensions: Option<&[String]>) -> Option<String> {
    let extensions = extensions?;
    let predicates = extensions
        .iter()
        .filter(|extension| extension.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .map(|extension| format!("lower(rel_path) LIKE '%.{}'", sql_string_literal(extension)))
        .collect::<Vec<_>>();
    or_predicates(predicates)
}

#[allow(dead_code)]
fn scope_filter_predicate(scope: Option<&str>) -> Option<String> {
    let scope = scope?;
    or_predicates(vec![
        format!("rel_path = '{}'", sql_string_literal(scope)),
        format!("rel_path LIKE '{}/%' ESCAPE '\\'", sql_like_literal(scope)),
    ])
}

fn or_predicates(predicates: Vec<String>) -> Option<String> {
    if predicates.is_empty() {
        return None;
    }
    Some(format!("({})", predicates.join(" OR ")))
}

fn sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[allow(dead_code)]
fn sql_like_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\'' => escaped.push_str("''"),
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[derive(Debug, Clone)]
struct IndexHealth {
    status: String,
    coverage_pct: f64,
    warnings: Vec<String>,
}

async fn index_health_for(
    root: &Path,
    config: &Config,
    cache: Option<&IndexHealthCache>,
) -> IndexHealth {
    match cache {
        Some(cache) => cache.get(root, config).await,
        None => index_health(root, config),
    }
}

async fn index_health_for_phase(
    root: &Path,
    config: &Config,
    cache: Option<&IndexHealthCache>,
    phase: IndexPhase,
) -> IndexHealth {
    let mut health = if phase == IndexPhase::Building {
        IndexHealth {
            status: phase.as_str().to_string(),
            coverage_pct: 0.0,
            warnings: Vec::new(),
        }
    } else {
        index_health_for(root, config, cache).await
    };
    apply_index_phase(&mut health, phase);
    health
}

fn apply_index_phase(health: &mut IndexHealth, phase: IndexPhase) {
    health.status = phase.as_str().to_string();
    if phase == IndexPhase::Building {
        health.coverage_pct = 0.0;
    }
    health
        .warnings
        .retain(|warning| !warning.starts_with("index_status is `"));
    match phase {
        IndexPhase::Building => {
            health.warnings.push(
                "index is building; results are unavailable until shallow indexing completes"
                    .to_string(),
            );
        }
        IndexPhase::Partial => {
            health.warnings.push(
                "index_status is `partial`; shallow keyword results may be incomplete".to_string(),
            );
        }
        IndexPhase::Full => {}
    }
}

fn index_health(root: &Path, config: &Config) -> IndexHealth {
    match calculate_index_health(root, config) {
        Ok(health) => health,
        Err(error) => IndexHealth {
            status: "unknown".to_string(),
            coverage_pct: 0.0,
            warnings: vec![format!("could not determine index coverage: {error}")],
        },
    }
}

fn calculate_index_health(root: &Path, config: &Config) -> Result<IndexHealth, String> {
    let (_root, files) =
        crate::cli::collect_index_files(root, config).map_err(|e| e.to_string())?;
    let detector = SecretDetector::new();
    let files = files
        .into_iter()
        .filter(|file| !detector.should_skip_file(&file.rel_path))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(IndexHealth {
            status: "empty".to_string(),
            coverage_pct: 0.0,
            warnings: vec!["no indexable files were discovered for this path".to_string()],
        });
    }

    let hash_store = HashStore::open(root, config).map_err(|e| e.to_string())?;
    let mut indexed = 0usize;
    let mut known = 0usize;
    for file in &files {
        if let Some(status) = hash_store
            .get_status(&file.rel_path)
            .map_err(|e| e.to_string())?
        {
            known += 1;
            let current_hash = hash_file(&root.join(&file.rel_path)).map_err(|e| e.to_string())?;
            let stored_hash = hash_store
                .get_hash(&file.rel_path)
                .map_err(|e| e.to_string())?;
            if status == FileStatus::Indexed
                && stored_hash.as_deref() == Some(current_hash.as_str())
            {
                indexed += 1;
            }
        }
    }

    let vector_coverage_pct = percentage(indexed, files.len());
    let text_index_ready = TextIndex::open_readonly(root, config)
        .map(|index| index.is_ready())
        .unwrap_or(false);
    let coverage_pct = if text_index_ready {
        vector_coverage_pct
    } else if vector_coverage_pct >= 100.0 {
        99.9
    } else {
        vector_coverage_pct
    };
    let status = if indexed == files.len() && text_index_ready {
        "full"
    } else if indexed > 0 || known > 0 {
        "partial"
    } else {
        "empty"
    };
    let mut warnings = Vec::new();
    if status != "full" {
        warnings.push(format!(
            "index_status is `{status}`; results may be incomplete"
        ));
    }
    if indexed == files.len() && !text_index_ready {
        warnings.push(
            "Tantivy text index is not fully backfilled; keyword and hybrid results may be incomplete"
                .to_string(),
        );
    }

    Ok(IndexHealth {
        status: status.to_string(),
        coverage_pct,
        warnings,
    })
}

fn result_confidence(results: &[HybridResult], min_relevance: f32) -> &'static str {
    let max_score = max_relevance_score(results);
    let Some(top_score) = results
        .first()
        .map(|result| normalized_relevance(result, max_score))
    else {
        return "low";
    };
    let above_threshold = results
        .iter()
        .filter(|result| normalized_relevance(result, max_score) >= min_relevance)
        .count();

    if top_score > 0.8 && above_threshold >= 3 {
        "high"
    } else if top_score >= 0.5 || above_threshold >= 2 {
        "medium"
    } else {
        "low"
    }
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 1000.0).round() / 10.0
}

fn max_relevance_score(results: &[HybridResult]) -> f32 {
    results
        .iter()
        .map(|result| result.relevance_score)
        .fold(0.0_f32, f32::max)
}

fn normalized_relevance(result: &HybridResult, max_score: f32) -> f32 {
    if max_score <= 0.0 {
        return 0.0;
    }
    (result.relevance_score / max_score).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        collections::HashSet,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use crate::chunker::{Chunk, Language};
    use crate::config::Config;
    use crate::embedder::Embedder;
    use crate::error::Result as VektorResult;
    use crate::vector_store::{ChunkRow, VectorStore};
    use serde_json::json;

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

        fn texts_embedded(&self) -> usize {
            self.texts.load(Ordering::SeqCst)
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

    async fn run_request_with_fake_index(
        request: IndexToolRequest,
        mut config: Config,
    ) -> Result<IndexStats, String> {
        let IndexToolRequest {
            path,
            force,
            extensions,
            embedding_backend,
        } = request;
        if let Some(backend) = embedding_backend {
            config.embedding.backend = backend;
        }

        let options = IndexOptions { force, extensions };
        let (root, files) =
            crate::cli::collect_index_files_with_options(Path::new(&path), &config, &options)
                .map_err(|e| e.to_string())?;
        let embedder = FakeEmbedder::new();
        let store = VectorStore::new(&root, &config, TEST_DIM, TEST_MODEL)
            .await
            .map_err(|e| e.to_string())?;
        let mut text_index =
            crate::text_index::TextIndex::new(&root, &config).map_err(|e| e.to_string())?;
        let was_text_index_ready = text_index.is_ready();
        let is_full_directory_path = Path::new(&path).is_dir() && options.extensions.is_none();
        let run_options = crate::cli::IndexRunOptions::from_index_options(
            &options,
            was_text_index_ready,
            is_full_directory_path,
        );

        crate::cli::index_path_with_embedder(
            &root,
            files,
            &config,
            &embedder,
            store,
            &mut text_index,
            run_options,
        )
        .await
        .map_err(|e| e.to_string())
    }

    struct IndexedFixture {
        _tempdir: tempfile::TempDir,
        repo: PathBuf,
        config: Config,
    }

    async fn indexed_fixture(files: &[(&str, &str)]) -> IndexedFixture {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        for (rel_path, content) in files {
            let path = repo.join(rel_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir parent");
            }
            std::fs::write(path, content).expect("write fixture file");
        }

        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut index_args = JsonObject::new();
        index_args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );
        run_index_tool(Some(index_args), |request| {
            run_request_with_fake_index(request, config.clone())
        })
        .await
        .expect("index should succeed");

        IndexedFixture {
            _tempdir: tempdir,
            repo,
            config,
        }
    }

    fn search_args(repo: &std::path::Path, query: &str) -> JsonObject {
        let mut args = JsonObject::new();
        args.insert("query".into(), Value::String(query.to_string()));
        args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );
        args
    }

    fn string_values(values: &[&str]) -> Value {
        Value::Array(
            values
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        )
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
    async fn index_codebase_honors_extensions_filter() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(repo.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write rs");
        std::fs::write(repo.join("notes.txt"), "plain notes\n").expect("write txt");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );
        args.insert(
            "extensions".into(),
            Value::Array(vec![Value::String("rs".into())]),
        );

        let response = run_index_tool(Some(args), |request| {
            run_request_with_fake_index(request, config.clone())
        })
        .await
        .expect("tool should succeed");

        assert_eq!(response["status"], "indexed");
        assert_eq!(response["files"], 1);
        assert_eq!(response["changed"], 1);
    }

    #[tokio::test]
    async fn index_codebase_normalizes_extension_filters() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(repo.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write rs");
        std::fs::write(repo.join("notes.txt"), "plain notes\n").expect("write txt");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );
        args.insert(
            "extensions".into(),
            Value::Array(vec![Value::String(".RS".into())]),
        );

        let response = run_index_tool(Some(args), |request| {
            run_request_with_fake_index(request, config.clone())
        })
        .await
        .expect("tool should succeed");

        assert_eq!(response["status"], "indexed");
        assert_eq!(response["files"], 1);
        assert_eq!(response["changed"], 1);
    }

    #[tokio::test]
    async fn keyword_search_works_after_filtered_index() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(
            repo.join("lib.rs"),
            "pub fn filtered_search_needle() -> bool {\n    true\n}\n",
        )
        .expect("write rs");
        std::fs::write(
            repo.join("helper.py"),
            "def unindexed_python():\n    return True\n",
        )
        .expect("write py");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut index_args = JsonObject::new();
        index_args.insert(
            "path".into(),
            Value::String(repo.to_string_lossy().into_owned()),
        );
        index_args.insert("extensions".into(), string_values(&["rs"]));
        run_index_tool(Some(index_args), |request| {
            run_request_with_fake_index(request, config.clone())
        })
        .await
        .expect("filtered index should succeed");

        let mut args = search_args(&repo, "filtered_search_needle");
        args.insert("mode".into(), Value::String("keyword".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let response = run_search_keyword_only(&request, &root, &config)
            .await
            .expect("filtered text index should be searchable");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "lib.rs");
        assert_eq!(response["metadata"]["index_status"], "partial");
    }

    #[tokio::test]
    async fn index_codebase_honors_embedding_backend_override() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(tempdir.path().to_string_lossy().into_owned()),
        );
        args.insert("embedding_backend".into(), Value::String("OpenAI".into()));

        let response = run_index_tool(Some(args), |request| async move {
            let mut config = Config::default();
            if let Some(backend) = request.embedding_backend {
                config.embedding.backend = backend;
            }
            assert_eq!(config.embedding.backend, "openai");
            Ok(IndexStats {
                files: 1,
                ..IndexStats::default()
            })
        })
        .await
        .expect("tool should succeed");

        assert_eq!(response["status"], "indexed");
        assert_eq!(response["files"], 1);
    }

    #[tokio::test]
    async fn index_codebase_rejects_invalid_optional_args() {
        let tempdir = tempfile::tempdir().expect("tempdir");

        let mut bad_extension = JsonObject::new();
        bad_extension.insert(
            "path".into(),
            Value::String(tempdir.path().to_string_lossy().into_owned()),
        );
        bad_extension.insert(
            "extensions".into(),
            Value::Array(vec![Value::String(".".into())]),
        );
        let error = run_index_tool(Some(bad_extension), |_request| async {
            Ok(IndexStats::default())
        })
        .await
        .expect_err("empty extension must fail");
        assert!(error.contains("extensions"), "{error}");

        let mut bad_backend = JsonObject::new();
        bad_backend.insert(
            "path".into(),
            Value::String(tempdir.path().to_string_lossy().into_owned()),
        );
        bad_backend.insert("embedding_backend".into(), Value::String("bogus".into()));
        let error = run_index_tool(Some(bad_backend), |_request| async {
            Ok(IndexStats::default())
        })
        .await
        .expect_err("unknown backend must fail");
        assert!(error.contains("embedding_backend"), "{error}");
    }

    #[tokio::test]
    async fn tool_requests_trim_and_reject_paths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let expected_path = tempdir.path().to_string_lossy().into_owned();
        let raw_path = format!(" {expected_path}\n");

        let mut index_args = JsonObject::new();
        index_args.insert("path".into(), Value::String(raw_path.clone()));
        let expected_index_path = expected_path.clone();
        let response = run_index_tool(Some(index_args), |request| async move {
            assert_eq!(request.path, expected_index_path);
            Ok(IndexStats::default())
        })
        .await
        .expect("trimmed index path should parse");
        assert_eq!(response["status"], "indexed");

        let mut search_args = JsonObject::new();
        search_args.insert("query".into(), Value::String("needle".into()));
        search_args.insert("path".into(), Value::String(raw_path.clone()));
        let request = parse_search_tool_request(Some(search_args)).expect("trimmed search path");
        assert_eq!(request.path, expected_path);

        let mut context_args = JsonObject::new();
        context_args.insert("query".into(), Value::String("needle".into()));
        context_args.insert("path".into(), Value::String(raw_path));
        let request = parse_context_tool_request(Some(context_args)).expect("trimmed context path");
        assert_eq!(request.path, expected_path);

        let mut blank_index = JsonObject::new();
        blank_index.insert("path".into(), Value::String(" \n\t".into()));
        let error = run_index_tool(Some(blank_index), |_request| async {
            Ok(IndexStats::default())
        })
        .await
        .expect_err("blank index path must fail");
        assert!(error.contains("path"), "{error}");

        let mut blank_search = JsonObject::new();
        blank_search.insert("query".into(), Value::String("needle".into()));
        blank_search.insert("path".into(), Value::String(" \n\t".into()));
        let response = handle_search_code(Some(blank_search)).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("path"),
            "blank search path should be a path validation error: {response}"
        );

        let mut blank_context = JsonObject::new();
        blank_context.insert("query".into(), Value::String("needle".into()));
        blank_context.insert("path".into(), Value::String(" \n\t".into()));
        let response = handle_get_context_for_prompt(Some(blank_context)).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("path"),
            "blank context path should be a path validation error: {response}"
        );
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
        // seam instead of the production `index_path_with_options` (which needs a model).
        let config_for_closure = config.clone();
        let response = run_index_tool(Some(args), |request| {
            run_request_with_fake_index(request, config_for_closure.clone())
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

    #[tokio::test]
    async fn search_code_returns_ranked_results() {
        let fixture = indexed_fixture(&[
            (
                "src/auth.rs",
                "pub fn validate_token(token: &str) -> bool {\n    token.contains(\"jwt\")\n}\n",
            ),
            (
                "src/db.rs",
                "pub fn connect_database() -> bool {\n    true\n}\n",
            ),
        ])
        .await;
        let mut args = search_args(&fixture.repo, "validate_token jwt");
        args.insert("top_k".into(), Value::Number(3.into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        assert!(
            response.get("status").is_none(),
            "success uses PRD envelope"
        );
        let results = response["results"].as_array().expect("results array");
        assert!(!results.is_empty(), "search returned no hits: {response}");
        assert_eq!(results[0]["file"], "src/auth.rs");
        assert_eq!(results[0]["language"], "rust");
        assert!(results[0]["lines"].as_str().expect("lines").contains('-'));
        assert!(results[0]["score"].is_number());
        let score = results[0]["score"].as_f64().expect("score");
        assert!(
            (0.0..=1.0).contains(&score),
            "wire scores must be normalized for PRD metadata: {response}"
        );
        assert!(
            results[0]["snippet"]
                .as_str()
                .expect("snippet")
                .contains("validate_token"),
            "result snippet must come from the real hybrid search path: {response}"
        );
        assert_eq!(response["metadata"]["mode"], "hybrid");
        assert_eq!(response["metadata"]["cache_hit"], false);
        assert_eq!(response["metadata"]["index_status"], "full");
        assert_eq!(response["metadata"]["index_coverage_pct"], 100.0);
        assert_eq!(
            embedder.texts_embedded(),
            1,
            "hybrid mode must embed the query through search_hybrid"
        );
    }

    #[tokio::test]
    async fn search_metadata_excludes_file_level_secret_skips_from_coverage() {
        let fixture = indexed_fixture(&[
            (
                "src/lib.rs",
                "pub fn healthneedle() -> bool {\n    true\n}\n",
            ),
            (".env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n"),
        ])
        .await;
        let args = search_args(&fixture.repo, "healthneedle");
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        assert_eq!(response["metadata"]["index_status"], "full");
        assert_eq!(response["metadata"]["index_coverage_pct"], 100.0);
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            warnings.is_empty(),
            "file-level secret skips should not create permanent incomplete-index warnings: {response}"
        );
    }

    #[tokio::test]
    async fn search_metadata_marks_changed_files_stale() {
        let fixture = indexed_fixture(&[(
            "src/lib.rs",
            "pub fn staleneedle() -> bool {\n    true\n}\n",
        )])
        .await;
        std::fs::write(
            fixture.repo.join("src/lib.rs"),
            "pub fn staleneedle() -> bool {\n    false\n}\n",
        )
        .expect("modify indexed file");
        let args = search_args(&fixture.repo, "staleneedle");
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        assert_eq!(response["metadata"]["index_status"], "partial");
        assert_eq!(response["metadata"]["index_coverage_pct"], 0.0);
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            !warnings.is_empty(),
            "stale indexed files should report incomplete coverage: {response}"
        );
    }

    #[tokio::test]
    async fn search_metadata_marks_unready_text_index_partial_even_with_current_hashes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        std::fs::write(
            repo.join("src/lib.rs"),
            "pub fn partialtextneedle() -> bool {\n    true\n}\n",
        )
        .expect("write rs");
        std::fs::write(
            repo.join("src/helper.py"),
            "def partialtextneedle_py():\n    return True\n",
        )
        .expect("write py");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let hash_store = HashStore::open(&repo, &config).expect("open hash store");
        for rel_path in ["src/lib.rs", "src/helper.py"] {
            let current_hash = hash_file(&repo.join(rel_path)).expect("hash file");
            hash_store
                .set_hash(rel_path, &current_hash, FileStatus::Indexed)
                .expect("set indexed hash");
        }

        let mut text_index = TextIndex::new(&repo, &config).expect("create text index");
        let chunks = vec![Chunk::new(
            "pub fn partialtextneedle() -> bool {\n    true\n}\n".to_string(),
            "src/lib.rs".to_string(),
            1,
            3,
            Some("partialtextneedle".to_string()),
            Some("function".to_string()),
            Some(Language::Rust),
        )];
        text_index.add_chunks(&chunks).expect("add partial chunks");
        text_index
            .commit_with_ready_marker(false)
            .expect("commit searchable but not globally ready");

        let health = calculate_index_health(&repo, &config).expect("calculate health");

        assert_eq!(health.status, "partial");
        assert!(
            health.coverage_pct < 100.0,
            "unready text index must not report full coverage"
        );
        assert!(
            health
                .warnings
                .iter()
                .any(|warning| warning.contains("not fully backfilled")),
            "partial text backfill should surface an explicit warning: {health:?}"
        );
    }

    #[tokio::test]
    async fn index_tool_success_invalidates_cached_index_health() {
        let fixture = indexed_fixture(&[(
            "src/lib.rs",
            "pub fn cachehealthneedle() -> bool {\n    true\n}\n",
        )])
        .await;
        let cache = IndexHealthCache::default();

        let cached = cache.get(&fixture.repo, &fixture.config).await;
        assert_eq!(cached.status, "full");

        std::fs::write(
            fixture.repo.join("src/new_file.rs"),
            "pub fn new_cachehealthneedle() -> bool {\n    true\n}\n",
        )
        .expect("write new unindexed file");
        let still_cached = cache.get(&fixture.repo, &fixture.config).await;
        assert_eq!(
            still_cached.status, "full",
            "precondition: health cache should hide the new file until invalidated"
        );

        let mut args = JsonObject::new();
        args.insert(
            "path".into(),
            Value::String(fixture.repo.to_string_lossy().into_owned()),
        );
        let query_cache = ContextQueryCache::default();
        let response = run_index_tool_with_cache_invalidation(
            Some(args),
            |_request| async {
                Ok(IndexStats {
                    files: 1,
                    ..IndexStats::default()
                })
            },
            Some(&cache),
            Some(&query_cache),
        )
        .await
        .expect("successful index tool should invalidate health cache");

        assert_eq!(response["status"], "indexed");
        let refreshed = cache.get(&fixture.repo, &fixture.config).await;
        assert_eq!(refreshed.status, "partial");
        assert!(
            refreshed.coverage_pct < 100.0,
            "cleared cache must recompute coverage against current files"
        );
    }

    #[tokio::test]
    async fn index_tool_success_invalidates_cached_context_queries() {
        let fixture = indexed_fixture(&[(
            "src/auth.rs",
            "pub fn cachequeryneedle() -> bool {\n    true\n}\n",
        )])
        .await;
        let args = search_args(&fixture.repo, "cachequeryneedle");
        let request = parse_context_tool_request(Some(args.clone())).expect("valid context args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();
        let health_cache = IndexHealthCache::default();
        let query_cache = ContextQueryCache::default();

        let first = run_context_with_embedder_cached(
            &request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("first context");
        let second = run_context_with_embedder_cached(
            &request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("second context");

        assert_eq!(first["metadata"]["cache_hit"], false);
        assert_eq!(second["metadata"]["cache_hit"], true);

        let mut index_args = JsonObject::new();
        index_args.insert(
            "path".into(),
            Value::String(fixture.repo.to_string_lossy().into_owned()),
        );
        let _ = run_index_tool_with_cache_invalidation(
            Some(index_args),
            |_request| async {
                Ok(IndexStats {
                    files: 1,
                    ..IndexStats::default()
                })
            },
            None,
            Some(&query_cache),
        )
        .await
        .expect("index tool should succeed");

        let after_invalidation = run_context_with_embedder_cached(
            &request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("context after index");

        assert_eq!(after_invalidation["metadata"]["cache_hit"], false);
    }

    #[tokio::test]
    async fn search_code_honors_filter_ext_and_bypass_cache() {
        let fixture = indexed_fixture(&[
            (
                "src/lib.rs",
                "pub fn sharedneedle_rust() -> bool {\n    true\n}\n",
            ),
            (
                "tools/helper.py",
                "def sharedneedle_python():\n    return True\n",
            ),
        ])
        .await;
        let mut args = search_args(&fixture.repo, "sharedneedle");
        args.insert("filter_ext".into(), string_values(&[".py"]));
        args.insert("bypass_cache".into(), Value::Bool(true));
        args.insert("top_k".into(), Value::Number(5.into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        let results = response["results"].as_array().expect("results array");
        assert!(
            !results.is_empty(),
            "filtered search returned no hits: {response}"
        );
        assert!(
            results
                .iter()
                .all(|result| result["file"].as_str().expect("file").ends_with(".py")),
            "filter_ext must be enforced after merge too: {response}"
        );
        assert_eq!(response["metadata"]["cache_hit"], false);
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.as_str().expect("warning").contains("query caching")),
            "bypass_cache should be accepted but honestly reported: {response}"
        );
    }

    #[tokio::test]
    async fn semantic_search_filter_ext_matches_uppercase_file_extension() {
        let fixture = indexed_fixture(&[(
            "src/Foo.RS",
            "pub fn upperextneedle() -> bool {\n    true\n}\n",
        )])
        .await;
        let mut args = search_args(&fixture.repo, "upperextneedle");
        args.insert("mode".into(), Value::String("semantic".into()));
        args.insert("filter_ext".into(), string_values(&["rs"]));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "src/Foo.RS");
    }

    #[tokio::test]
    async fn search_code_filter_ext_recovers_beyond_initial_keyword_window() {
        let mut paths = Vec::new();
        let mut contents = Vec::new();
        for idx in 0..24 {
            paths.push(format!("src/high_{idx}.rs"));
            contents.push(
                "pub fn rarefilterneedle_rust() -> bool {\n    rarefilterneedle && rarefilterneedle\n}\n"
                    .to_string(),
            );
        }
        paths.push("tools/low.py".to_string());
        contents.push("def rarefilterneedle_python():\n    return True\n".to_string());
        let files = paths
            .iter()
            .zip(contents.iter())
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect::<Vec<_>>();
        let fixture = indexed_fixture(&files).await;

        let mut args = search_args(&fixture.repo, "rarefilterneedle");
        args.insert("mode".into(), Value::String("keyword".into()));
        args.insert("filter_ext".into(), string_values(&["py"]));
        args.insert("top_k".into(), Value::Number(1.into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(
            results.len(),
            1,
            "top_k should cap filtered hits: {response}"
        );
        assert_eq!(results[0]["file"], "tools/low.py");
    }

    #[tokio::test]
    async fn keyword_search_does_not_build_embedder() {
        let fixture = indexed_fixture(&[(
            "src/auth.rs",
            "pub fn keywordonlyneedle_auth() -> bool {\n    true\n}\n",
        )])
        .await;
        let mut args = search_args(&fixture.repo, "keywordonlyneedle");
        args.insert("mode".into(), Value::String("keyword".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let mut config = fixture.config.clone();
        config.embedding.backend = "openai".to_string();
        config.embedding.openai_api_key.clear();
        config.embedding.fallback_to_onnx = false;

        let response = run_search_request(
            &request,
            &root,
            &mut config,
            &EmbedderCache::default(),
            &IndexHealthCache::default(),
        )
        .await
        .expect("keyword search should not require embedder construction");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "src/auth.rs");
        assert_eq!(response["metadata"]["mode"], "keyword");
    }

    #[tokio::test]
    async fn building_search_returns_empty_without_building_embedder() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(
            repo.join("lib.rs"),
            "pub fn buildingneedle() -> bool {\n    true\n}\n",
        )
        .expect("write");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        config.embedding.backend = "openai".to_string();
        config.embedding.openai_api_key.clear();
        config.embedding.fallback_to_onnx = false;

        let args = search_args(&repo, "buildingneedle");
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let tracker = IndexStatusTracker::default();

        let response = run_search_request_with_status(
            &request,
            &root,
            &mut config,
            &EmbedderCache::default(),
            &IndexHealthCache::default(),
            Some(&tracker),
        )
        .await
        .expect("building search should not require embedder construction");

        assert!(
            response["results"].as_array().expect("results").is_empty(),
            "building phase must return no results: {response}"
        );
        assert_eq!(response["metadata"]["index_status"], "building");
        assert!(
            response["metadata"]["missing_context_warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning.as_str().expect("warning").contains("building")),
            "building status must be explicit: {response}"
        );
    }

    #[tokio::test]
    async fn partial_search_uses_shallow_keyword_tier_without_embedder() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        std::fs::write(
            repo.join("src/lib.rs"),
            "pub fn partialshallowneedle() -> bool {\n    true\n}\n",
        )
        .expect("write");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        config.embedding.backend = "openai".to_string();
        config.embedding.openai_api_key.clear();
        config.embedding.fallback_to_onnx = false;
        ShallowIndexer::build(&repo, &config).expect("build shallow");

        let args = search_args(&repo, "partialshallowneedle");
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let tracker = IndexStatusTracker::default();

        let response = run_search_request_with_status(
            &request,
            &root,
            &mut config,
            &EmbedderCache::default(),
            &IndexHealthCache::default(),
            Some(&tracker),
        )
        .await
        .expect("partial search should not require embedder construction");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "src/lib.rs");
        assert!(
            results[0]["snippet"]
                .as_str()
                .expect("snippet")
                .contains("partialshallowneedle"),
            "partial results must use stored shallow content: {response}"
        );
        assert_eq!(response["metadata"]["mode"], "keyword");
        assert_eq!(response["metadata"]["index_status"], "partial");
    }

    #[tokio::test]
    async fn search_code_with_config_uses_served_index_data_dir() {
        let fixture = indexed_fixture(&[(
            "src/configured.rs",
            "pub fn configuredneedle() -> bool {\n    true\n}\n",
        )])
        .await;
        let mut args = search_args(&fixture.repo, "configuredneedle");
        args.insert("mode".into(), Value::String("keyword".into()));

        let response = handle_search_code_with_config(Some(args), fixture.config.clone()).await;

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "src/configured.rs");
        assert_eq!(response["metadata"]["mode"], "keyword");
    }

    #[tokio::test]
    async fn keyword_search_vector_only_index_requires_tantivy_backfill() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        VectorStore::new(&repo, &config, TEST_DIM, TEST_MODEL)
            .await
            .expect("seed vector-only metadata");

        let mut args = search_args(&repo, "needle");
        args.insert("mode".into(), Value::String("keyword".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");

        let error = run_search_keyword_only(&request, &root, &config)
            .await
            .expect_err("keyword search should require a committed Tantivy index");

        assert!(error.to_string().contains("vektor index"), "{error}");
        assert!(
            !TextIndex::exists(&root, &config).expect("check Tantivy existence"),
            "keyword search must not create an empty Tantivy index"
        );
    }

    #[tokio::test]
    async fn search_file_path_uses_same_root_as_index_file_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        let src = repo.join("src");
        std::fs::create_dir_all(&src).expect("mkdir src");
        let file = src.join("lib.rs");
        std::fs::write(&file, "pub fn fileonlyneedle() -> bool {\n    true\n}\n")
            .expect("write file");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut index_args = JsonObject::new();
        index_args.insert(
            "path".into(),
            Value::String(file.to_string_lossy().into_owned()),
        );
        run_index_tool(Some(index_args), |request| {
            run_request_with_fake_index(request, config.clone())
        })
        .await
        .expect("file index should succeed");

        let mut args = search_args(&file, "fileonlyneedle");
        args.insert("mode".into(), Value::String("keyword".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        assert_eq!(root, src.canonicalize().expect("canonical src"));

        let response = run_search_keyword_only(&request, &root, &config)
            .await
            .expect("searching the same file path should find its parent-scoped index");
        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "lib.rs");
    }

    #[tokio::test]
    async fn search_restores_backend_from_index_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        config.embedding.backend = "onnx".to_string();
        VectorStore::new(&repo, &config, TEST_DIM, &config.embedding.openai_model)
            .await
            .expect("seed openai-indexed metadata");

        apply_indexed_backend(&repo, &mut config).expect("restore backend");

        assert_eq!(config.embedding.backend, "openai");
    }

    #[tokio::test]
    async fn semantic_search_unindexed_path_does_not_create_vector_store() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();

        let mut args = search_args(&repo, "needle");
        args.insert("mode".into(), Value::String("semantic".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let error = run_search_with_embedder(&request, &root, &config, &embedder)
            .await
            .expect_err("unindexed semantic search should fail read-only");

        assert!(error.contains("run `vektor index` first"), "{error}");
        assert!(
            VectorStore::load_meta(&root, &config)
                .expect("load meta")
                .is_none(),
            "read-only search must not create vector-store metadata"
        );
    }

    #[tokio::test]
    async fn semantic_search_unindexed_path_does_not_build_embedder() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        config.embedding.backend = "openai".to_string();
        config.embedding.openai_api_key.clear();
        config.embedding.fallback_to_onnx = false;

        let mut args = search_args(&repo, "needle");
        args.insert("mode".into(), Value::String("semantic".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");

        let error = run_search_request(
            &request,
            &root,
            &mut config,
            &EmbedderCache::default(),
            &IndexHealthCache::default(),
        )
        .await
        .expect_err("unindexed semantic search should fail before embedder construction");

        assert!(error.contains("vektor index"), "{error}");
        assert!(!error.contains("api key"), "{error}");
        assert!(
            VectorStore::load_meta(&root, &config)
                .expect("load meta")
                .is_none(),
            "read-only search must not create vector-store metadata"
        );
    }

    #[tokio::test]
    async fn semantic_search_vector_only_index_does_not_require_tantivy() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        let mut store = VectorStore::new(&repo, &config, TEST_DIM, TEST_MODEL)
            .await
            .expect("create vector store");
        store
            .insert_chunks(&[ChunkRow {
                id: "semantic-vector-only".to_string(),
                content_hash: "semantic-hash".to_string(),
                vector: vec![1.0; TEST_DIM],
                rel_path: "src/lib.rs".to_string(),
                start_line: 1,
                end_line: 3,
                symbol_name: Some("semanticneedle".to_string()),
                symbol_type: Some("function".to_string()),
                language: "rust".to_string(),
                content: "pub fn semanticneedle() -> bool { true }".to_string(),
                last_modified: 1_700_000_000,
            }])
            .await
            .expect("seed vector row");
        assert!(!TextIndex::exists(&repo, &config).expect("check Tantivy existence"));

        let mut args = search_args(&repo, "semanticneedle");
        args.insert("mode".into(), Value::String("semantic".into()));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &config, &embedder)
            .await
            .expect("semantic search should not require Tantivy");

        let results = response["results"].as_array().expect("results array");
        assert_eq!(results[0]["file"], "src/lib.rs");
        assert_eq!(response["metadata"]["mode"], "semantic");
        assert!(!TextIndex::exists(&repo, &config).expect("check Tantivy existence"));
    }

    #[tokio::test]
    async fn search_code_normalizes_filter_ext() {
        let fixture = indexed_fixture(&[
            (
                "src/lib.rs",
                "pub fn normalizeneedle_rust() -> bool {\n    true\n}\n",
            ),
            (
                "tools/helper.py",
                "def normalizeneedle_python():\n    return True\n",
            ),
        ])
        .await;
        let mut args = search_args(&fixture.repo, "normalizeneedle");
        args.insert("filter_ext".into(), string_values(&[".RS"]));
        let request = parse_search_tool_request(Some(args)).expect("valid search args");
        assert_eq!(request.filter_ext.as_deref().expect("filter ext"), ["rs"]);
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_search_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("search should succeed");

        let results = response["results"].as_array().expect("results array");
        assert!(
            !results.is_empty(),
            "filtered search returned no hits: {response}"
        );
        assert!(
            results
                .iter()
                .all(|result| result["file"].as_str().expect("file").ends_with(".rs")),
            "dotted/uppercase filter_ext should normalize to bare lowercase: {response}"
        );
    }

    #[tokio::test]
    async fn get_context_returns_context_package() {
        let fixture = indexed_fixture(&[(
            "src/auth.rs",
            "pub fn validate_token(token: &str) -> bool {\n    token.contains(\"jwt\")\n}\n",
        )])
        .await;
        let mut args = search_args(&fixture.repo, "validate_token jwt");
        args.insert("token_budget".into(), Value::Number(1_000.into()));
        args.insert("include_related".into(), Value::Bool(false));
        let request = parse_context_tool_request(Some(args)).expect("valid context args");
        assert_eq!(request.min_relevance, 0.5);
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_context_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("context should succeed");

        let context = response["context"].as_array().expect("context array");
        assert!(!context.is_empty(), "context returned no hits: {response}");
        assert_eq!(context[0]["file"], "src/auth.rs");
        assert_eq!(context[0]["source"], "search");
        assert!(context[0]["relevance"].is_number());
        let relevance = context[0]["relevance"].as_f64().expect("relevance");
        assert!(
            (0.5..=1.0).contains(&relevance),
            "min_relevance must compare normalized PRD relevance: {response}"
        );
        assert!(
            context[0]["content"]
                .as_str()
                .expect("content")
                .contains("validate_token")
        );
        assert_eq!(response["metadata"]["files_included"], 1);
        assert_eq!(response["metadata"]["chunks_returned"], context.len());
        assert_eq!(response["metadata"]["chunks_deduplicated"], 0);
        assert_eq!(response["metadata"]["cache_hit"], false);
        assert_eq!(response["metadata"]["index_status"], "full");
        assert!(response["metadata"]["total_tokens"].is_number());
        assert!(response["metadata"]["budget_gap_reason"].is_string());
        assert!(response["metadata"]["clusters"].is_array());
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.as_str().expect("warning").contains("Phase 4")),
            "real assembler should not emit Phase 4 deferral warnings: {response}"
        );
    }

    #[tokio::test]
    async fn building_context_returns_empty_package_without_building_embedder() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir repo");
        std::fs::write(
            repo.join("lib.rs"),
            "pub fn contextbuildingneedle() -> bool {\n    true\n}\n",
        )
        .expect("write");
        let mut config = Config::default();
        config.index.data_dir = tempdir.path().join("data").to_string_lossy().into_owned();
        config.embedding.backend = "openai".to_string();
        config.embedding.openai_api_key.clear();
        config.embedding.fallback_to_onnx = false;

        let mut args = search_args(&repo, "contextbuildingneedle");
        args.insert("include_related".into(), Value::Bool(false));
        let tracker = IndexStatusTracker::default();
        let response = run_context_tool_with_status(
            Some(args),
            config,
            &EmbedderCache::default(),
            &IndexHealthCache::default(),
            &ContextQueryCache::default(),
            Some(&tracker),
        )
        .await
        .expect("building context should not require embedder construction");

        assert!(
            response["context"].as_array().expect("context").is_empty(),
            "building phase must return an empty context package: {response}"
        );
        assert_eq!(response["metadata"]["index_status"], "building");
        assert_eq!(
            response["metadata"]["budget_gap_reason"],
            "index_incomplete"
        );
    }

    #[tokio::test]
    async fn get_context_honors_wire_controls_with_real_assembler() {
        let fixture = indexed_fixture(&[
            (
                "src/auth.rs",
                "pub fn controlneedle_auth() -> bool {\n    true\n}\n",
            ),
            (
                "src/payments.rs",
                "pub fn controlneedle_payments() -> bool {\n    true\n}\n",
            ),
            (
                "README.md",
                "# controlneedle docs\n\nThis document should be filtered.\n",
            ),
        ])
        .await;
        let mut args = search_args(&fixture.repo, "controlneedle");
        args.insert("max_files".into(), Value::Number(1.into()));
        args.insert("min_relevance".into(), json!(0.5));
        args.insert("include_docs".into(), Value::Bool(false));
        args.insert("scope".into(), Value::String("src".to_string()));
        args.insert("token_budget".into(), Value::Number(1.into()));
        args.insert("include_related".into(), Value::Bool(true));
        args.insert("bypass_cache".into(), Value::Bool(true));
        let request = parse_context_tool_request(Some(args)).expect("valid context args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();

        let response = run_context_with_embedder(&request, &root, &fixture.config, &embedder)
            .await
            .expect("context should succeed");

        let context = response["context"].as_array().expect("context array");
        assert!(!context.is_empty(), "context returned no hits: {response}");
        let files = context
            .iter()
            .map(|item| item["file"].as_str().expect("file"))
            .collect::<HashSet<_>>();
        assert!(
            files.len() <= 1,
            "max_files must cap distinct files: {response}"
        );
        assert!(
            files
                .iter()
                .all(|file| file.starts_with("src/") && !file.ends_with(".md")),
            "scope and include_docs=false must shape returned context: {response}"
        );
        assert_eq!(response["metadata"]["files_included"], files.len());
        assert!(
            response["metadata"]["total_tokens"]
                .as_u64()
                .expect("total tokens")
                <= 1,
            "real assembler must enforce token_budget: {response}"
        );
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.as_str().expect("warning").contains("deferred")),
            "Phase 4 deferral warnings should be gone: {response}"
        );
    }

    #[tokio::test]
    async fn get_context_cache_hit_and_bypass() {
        let fixture = indexed_fixture(&[(
            "src/auth.rs",
            "pub fn cachenoodle_auth() -> bool {\n    true\n}\n",
        )])
        .await;
        let args = search_args(&fixture.repo, "cachenoodle");
        let request = parse_context_tool_request(Some(args.clone())).expect("valid context args");
        let root = canonical_project_root(&request.path).expect("canonical root");
        let embedder = FakeEmbedder::new();
        let health_cache = IndexHealthCache::default();
        let query_cache = ContextQueryCache::default();

        let first = run_context_with_embedder_cached(
            &request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("first context");
        let second = run_context_with_embedder_cached(
            &request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("second context");

        let mut bypass_args = args;
        bypass_args.insert("bypass_cache".into(), Value::Bool(true));
        let bypass_request =
            parse_context_tool_request(Some(bypass_args)).expect("valid bypass args");
        let bypass = run_context_with_embedder_cached(
            &bypass_request,
            &root,
            &fixture.config,
            &embedder,
            Some(&health_cache),
            Some(&query_cache),
        )
        .await
        .expect("bypass context");

        assert_eq!(first["metadata"]["cache_hit"], false);
        assert_eq!(second["metadata"]["cache_hit"], true);
        assert_eq!(bypass["metadata"]["cache_hit"], false);
    }

    #[test]
    fn scope_filter_predicate_escapes_like_wildcards() {
        let predicate = scope_filter_predicate(Some("src/my_mod%")).expect("scope predicate");

        assert!(
            predicate.contains("rel_path = 'src/my_mod%'"),
            "exact predicate should remain SQL literal escaped: {predicate}"
        );
        assert!(
            predicate.contains("rel_path LIKE 'src/my\\_mod\\%/%' ESCAPE '\\'"),
            "LIKE predicate must treat `_` and `%` as literal path characters: {predicate}"
        );
    }

    #[tokio::test]
    async fn handlers_reject_bad_args_as_json_error() {
        let response = handle_search_code(None).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("query"),
            "error must mention required search arguments: {response}"
        );

        let mut bad_search = JsonObject::new();
        bad_search.insert("query".into(), Value::String("hello".into()));
        bad_search.insert("path".into(), Value::String(".".into()));
        bad_search.insert("filter_ext".into(), string_values(&[""]));
        let response = handle_search_code(Some(bad_search)).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("filter_ext"),
            "bad filter_ext should be a JSON error: {response}"
        );

        let mut bad_context = JsonObject::new();
        bad_context.insert("query".into(), Value::String("hello".into()));
        bad_context.insert("path".into(), Value::String(".".into()));
        bad_context.insert("min_relevance".into(), json!(2.0));
        let response = handle_get_context_for_prompt(Some(bad_context)).await;
        assert_eq!(response["status"], "error");
        assert!(
            response["error"]
                .as_str()
                .expect("error string")
                .contains("min_relevance"),
            "bad context args should be a JSON error: {response}"
        );
    }
}
