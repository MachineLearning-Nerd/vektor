use std::{
    collections::HashSet,
    future::Future,
    path::{Path, PathBuf},
    time::Instant,
};

use rmcp::model::JsonObject;
use serde_json::{Value, json};

use crate::{
    cli::{IndexOptions, IndexStats},
    config::Config,
    embedder::{Embedder, build_embedder},
    search::hybrid::{HybridResult, HybridSearchConfig, SearchMode, search_hybrid},
    state::{FileStatus, HashStore},
    text_index::TextIndex,
    vector_store::VectorStore,
};

const FILTERED_SEARCH_CANDIDATE_LIMIT: usize = 10_000;
const MIN_FILTERED_SEARCH_CANDIDATES: usize = 1_024;

/// Build or refresh the local codebase index.
///
/// Async because it builds the embedder + LanceDB/Tantivy stores and runs the
/// shared index core. Reads the required `path` argument and optional
/// `force_full`, `extensions`, and `embedding_backend` arguments from the MCP
/// tool arguments, loads the default [`Config`], and calls the SAME
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
pub async fn handle_index_codebase(args: Option<JsonObject>) -> Value {
    // Production indexer: load default config + run the shared CLI index core.
    let result = run_index_tool(args, |request| async move {
        let mut config = Config::load(None).map_err(|e| e.to_string())?;
        if let Some(backend) = request.embedding_backend {
            config.embedding.backend = backend;
        }

        let options = IndexOptions {
            force: request.force,
            extensions: request.extensions,
        };
        crate::cli::index_path_with_options(Path::new(&request.path), &config, options)
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
/// `index_path_with_options` (which needs a downloaded model). The production
/// handler passes the real shared core.
async fn run_index_tool<F, Fut>(args: Option<JsonObject>, indexer: F) -> Result<Value, String>
where
    F: FnOnce(IndexToolRequest) -> Fut,
    Fut: Future<Output = Result<IndexStats, String>>,
{
    let args = args.ok_or_else(|| "missing arguments: `path` is required".to_string())?;

    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `path` argument".to_string())?
        .to_string();

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

pub async fn handle_search_code(args: Option<JsonObject>) -> Value {
    let result = run_search_tool(args).await;

    match result {
        Ok(value) => value,
        Err(message) => {
            tracing::error!(error = %message, "search_code failed");
            json!({ "status": "error", "error": message })
        }
    }
}

pub async fn handle_get_context_for_prompt(args: Option<JsonObject>) -> Value {
    let result = run_context_tool(args).await;

    match result {
        Ok(value) => value,
        Err(message) => {
            tracing::error!(error = %message, "get_context_for_prompt failed");
            json!({ "status": "error", "error": message })
        }
    }
}

async fn run_search_tool(args: Option<JsonObject>) -> Result<Value, String> {
    let request = parse_search_tool_request(args)?;
    let root = canonical_project_root(&request.path)?;
    let config = Config::load(None).map_err(|e| e.to_string())?;
    let embedder = build_embedder(&config).map_err(|e| e.to_string())?;

    run_search_with_embedder(&request, &root, &config, embedder.as_ref()).await
}

async fn run_search_with_embedder(
    request: &SearchToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
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
    } = index_health(root, config);
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

async fn run_context_tool(args: Option<JsonObject>) -> Result<Value, String> {
    let request = parse_context_tool_request(args)?;
    let root = canonical_project_root(&request.path)?;
    let config = Config::load(None).map_err(|e| e.to_string())?;
    let embedder = build_embedder(&config).map_err(|e| e.to_string())?;

    run_context_with_embedder(&request, &root, &config, embedder.as_ref()).await
}

async fn run_context_with_embedder(
    request: &ContextToolRequest,
    root: &Path,
    config: &Config,
    embedder: &dyn Embedder,
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

    let filter = scope_filter_predicate(request.scope.as_deref());
    let search_limit = expanded_search_limit(request.max_files.saturating_mul(4).max(8), true);
    let (results, search_time_ms) = search_project(
        &request.query,
        SearchMode::Hybrid,
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
            filter_ext: None,
            min_relevance: Some(request.min_relevance),
            include_docs: request.include_docs,
            scope: request.scope.as_deref(),
            max_files: Some(request.max_files),
            limit: None,
        },
    );
    let health = index_health(root, config);

    Ok(context_results_to_json(
        &results,
        request,
        search_time_ms,
        health,
    ))
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
    let store = VectorStore::new(root, config, embedder.dim(), embedder.name()).await?;
    let text_index = TextIndex::new(root, config)?;
    let mut search_config = HybridSearchConfig::new(mode, top_k);
    search_config.filter = filter;
    let results = search_hybrid(query, &search_config, &store, &text_index, embedder).await?;

    Ok((results, started_at.elapsed().as_millis()))
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

    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `path` argument".to_string())?
        .to_string();

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

    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or non-string `path` argument".to_string())?
        .to_string();

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
    Path::new(path)
        .canonicalize()
        .map_err(|e| format!("invalid `path` argument `{path}`: {e}"))
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

fn context_results_to_json(
    results: &[HybridResult],
    request: &ContextToolRequest,
    search_time_ms: u128,
    health: IndexHealth,
) -> Value {
    let files_included = distinct_file_count(results);
    let total_tokens = estimate_total_tokens(results);
    let budget_used_pct = percentage(total_tokens, request.token_budget);
    let IndexHealth {
        status,
        coverage_pct,
        mut warnings,
    } = health;
    let max_score = max_relevance_score(results);

    warnings.push(
        "Phase 4 returns naive search chunks only; deduplication and token-budget allocation are deferred to Phase 5"
            .to_string(),
    );
    if request.include_related {
        warnings.push(
            "`include_related` was accepted, but related expansion is deferred to Phase 5"
                .to_string(),
        );
    }
    if request.bypass_cache {
        warnings.push(
            "`bypass_cache` was accepted, but Phase 4 does not implement query caching".to_string(),
        );
    }
    if total_tokens > request.token_budget {
        warnings.push(
            "`token_budget` was accepted, but Phase 4 does not trim or allocate chunks to fit it"
                .to_string(),
        );
    }

    json!({
        "context": results
            .iter()
            .map(|result| context_result_to_json(result, max_score))
            .collect::<Vec<_>>(),
        "metadata": {
            "files_included": files_included,
            "total_tokens": total_tokens,
            "budget_used_pct": budget_used_pct,
            "chunks_returned": results.len(),
            "chunks_deduplicated": 0,
            "search_time_ms": search_time_ms,
            "cache_hit": false,
            "index_status": status,
            "index_coverage_pct": coverage_pct,
            "result_confidence": result_confidence(results, request.min_relevance),
            "budget_gap_reason": budget_gap_reason(&status),
            "missing_context_warnings": warnings,
            "suggested_action": suggested_action(results, &status),
            "clusters": [],
        }
    })
}

fn context_result_to_json(result: &HybridResult, max_score: f32) -> Value {
    json!({
        "file": result.rel_path.as_str(),
        "lines": line_range(result),
        "symbol": result.symbol_name.as_deref(),
        "type": result.symbol_type.as_deref(),
        "language": result.language.as_str(),
        "relevance": normalized_relevance(result, max_score),
        "source": "search",
        "reason": "Primary Phase 4 search result for the query",
        "content": result.content.as_str(),
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
        .map(|extension| format!("rel_path LIKE '%.{}'", sql_string_literal(extension)))
        .collect::<Vec<_>>();
    or_predicates(predicates)
}

fn scope_filter_predicate(scope: Option<&str>) -> Option<String> {
    let scope = scope?;
    or_predicates(vec![
        format!("rel_path = '{}'", sql_string_literal(scope)),
        format!("rel_path LIKE '{}/%'", sql_string_literal(scope)),
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

#[derive(Debug)]
struct IndexHealth {
    status: String,
    coverage_pct: f64,
    warnings: Vec<String>,
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
            if status == FileStatus::Indexed {
                indexed += 1;
            }
        }
    }

    let coverage_pct = percentage(indexed, files.len());
    let status = if indexed == files.len() {
        "full"
    } else if indexed > 0 || known > 0 {
        "partial"
    } else {
        "empty"
    };
    let warnings = if status == "full" {
        Vec::new()
    } else {
        vec![format!(
            "index_status is `{status}`; results may be incomplete"
        )]
    };

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

fn distinct_file_count(results: &[HybridResult]) -> usize {
    results
        .iter()
        .map(|result| result.rel_path.as_str())
        .collect::<HashSet<_>>()
        .len()
}

fn estimate_total_tokens(results: &[HybridResult]) -> usize {
    results
        .iter()
        .map(|result| result.content.len().div_ceil(4))
        .sum()
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 1000.0).round() / 10.0
}

fn budget_gap_reason(index_status: &str) -> Value {
    if index_status != "full" {
        return json!("index_incomplete");
    }
    Value::Null
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

fn suggested_action(results: &[HybridResult], index_status: &str) -> Value {
    if index_status != "full" {
        return json!(
            "Run index_codebase to refresh the project index before relying on this context"
        );
    }
    if results.is_empty() {
        return json!("Broaden the query, lower min_relevance, or adjust scope");
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        collections::HashSet,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use crate::config::Config;
    use crate::embedder::Embedder;
    use crate::error::Result as VektorResult;
    use crate::vector_store::VectorStore;
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

        crate::cli::index_path_with_embedder(
            &root,
            files,
            &config,
            &embedder,
            store,
            &mut text_index,
            options.force,
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
        assert!(response["metadata"]["budget_gap_reason"].is_null());
        assert!(response["metadata"]["clusters"].is_array());
    }

    #[tokio::test]
    async fn get_context_honors_phase4_wire_controls() {
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
        let warnings = response["metadata"]["missing_context_warnings"]
            .as_array()
            .expect("warnings");
        assert!(
            warnings.iter().any(|warning| warning
                .as_str()
                .expect("warning")
                .contains("related expansion")),
            "include_related deferral must be surfaced: {response}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.as_str().expect("warning").contains("query caching")),
            "bypass_cache deferral must be surfaced: {response}"
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
