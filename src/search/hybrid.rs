use std::collections::HashMap;

use crate::{
    embedder::Embedder,
    error::{Result, VektorError},
    search::{
        rrf::{RRF_K, RankedId, rrf_fuse},
        synonyms::SynonymExpander,
        weights::AdaptiveWeights,
    },
    text_index::{KeywordHit, TextIndex},
    vector_store::{SearchResult as VectorSearchResult, VectorStore},
};

const DEFAULT_CANDIDATE_MULTIPLIER: usize = 4;
const DEFAULT_MAX_CANDIDATES: usize = 200;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SearchMode {
    #[default]
    Hybrid,
    Semantic,
    Keyword,
}

impl SearchMode {
    #[allow(dead_code)]
    pub(crate) fn parse(mode: &str) -> Result<Self> {
        match mode {
            "hybrid" => Ok(Self::Hybrid),
            "semantic" => Ok(Self::Semantic),
            "keyword" => Ok(Self::Keyword),
            other => Err(VektorError::Config(format!(
                "unsupported search mode `{other}`; expected `hybrid`, `semantic`, or `keyword`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HybridSearchConfig {
    pub(crate) mode: SearchMode,
    pub(crate) top_k: usize,
    pub(crate) filter: Option<String>,
    pub(crate) candidate_multiplier: usize,
    pub(crate) max_candidates: usize,
}

impl HybridSearchConfig {
    pub(crate) fn new(mode: SearchMode, top_k: usize) -> Self {
        Self {
            mode,
            top_k,
            filter: None,
            candidate_multiplier: DEFAULT_CANDIDATE_MULTIPLIER,
            max_candidates: DEFAULT_MAX_CANDIDATES,
        }
    }

    fn candidate_limit(&self) -> usize {
        if self.top_k == 0 {
            return 0;
        }

        let multiplier = self.candidate_multiplier.max(1);
        let max_candidates = self.max_candidates.max(self.top_k);
        self.top_k.saturating_mul(multiplier).min(max_candidates)
    }
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self::new(SearchMode::Hybrid, 10)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HybridResult {
    pub(crate) chunk_id: String,
    pub(crate) rel_path: String,
    pub(crate) start_line: u64,
    pub(crate) end_line: u64,
    pub(crate) symbol_name: Option<String>,
    pub(crate) symbol_type: Option<String>,
    pub(crate) language: String,
    pub(crate) content: String,
    pub(crate) relevance_score: f32,
    pub(crate) semantic_score: Option<f32>,
    pub(crate) keyword_score: Option<f32>,
    pub(crate) last_modified: i64,
}

#[async_trait::async_trait]
trait SemanticRetriever: Sync {
    async fn search_semantic(
        &self,
        query_vec: &[f32],
        top_k: usize,
        filter: Option<&str>,
    ) -> Result<Vec<VectorSearchResult>>;

    async fn hydrate_chunks_by_id(
        &self,
        chunk_ids: &[String],
    ) -> Result<HashMap<String, VectorSearchResult>>;
}

trait KeywordRetriever: Sync {
    fn search_keyword(&self, query: &str, top_k: usize) -> Result<Vec<KeywordHit>>;
}

#[async_trait::async_trait]
impl SemanticRetriever for VectorStore {
    async fn search_semantic(
        &self,
        query_vec: &[f32],
        top_k: usize,
        filter: Option<&str>,
    ) -> Result<Vec<VectorSearchResult>> {
        self.search(query_vec, top_k, filter).await
    }

    async fn hydrate_chunks_by_id(
        &self,
        chunk_ids: &[String],
    ) -> Result<HashMap<String, VectorSearchResult>> {
        self.chunks_by_ids(chunk_ids).await
    }
}

impl KeywordRetriever for TextIndex {
    fn search_keyword(&self, query: &str, top_k: usize) -> Result<Vec<KeywordHit>> {
        self.search(query, top_k)
    }
}

pub(crate) async fn search_hybrid(
    query: &str,
    config: &HybridSearchConfig,
    store: &VectorStore,
    text_index: &TextIndex,
    embedder: &dyn Embedder,
) -> Result<Vec<HybridResult>> {
    search_hybrid_with_retrievers(query, config, store, text_index, embedder).await
}

pub(crate) async fn search_keyword_only(
    query: &str,
    config: &HybridSearchConfig,
    store: &VectorStore,
    text_index: &TextIndex,
) -> Result<Vec<HybridResult>> {
    let mut config = config.clone();
    config.mode = SearchMode::Keyword;
    search_hybrid_with_optional_embedder(query, &config, store, text_index, None).await
}

pub(crate) async fn search_semantic_only(
    query: &str,
    config: &HybridSearchConfig,
    store: &VectorStore,
    embedder: &dyn Embedder,
) -> Result<Vec<HybridResult>> {
    let query = query.trim();
    if query.is_empty() || config.top_k == 0 {
        return Ok(Vec::new());
    }

    let hits = semantic_hits(
        query,
        config.top_k,
        config.filter.as_deref(),
        store,
        embedder,
    )
    .await?;
    Ok(hits
        .into_iter()
        .take(config.top_k)
        .map(semantic_result)
        .collect())
}

async fn search_hybrid_with_retrievers(
    query: &str,
    config: &HybridSearchConfig,
    semantic: &dyn SemanticRetriever,
    keyword: &dyn KeywordRetriever,
    embedder: &dyn Embedder,
) -> Result<Vec<HybridResult>> {
    search_hybrid_with_optional_embedder(query, config, semantic, keyword, Some(embedder)).await
}

async fn search_hybrid_with_optional_embedder(
    query: &str,
    config: &HybridSearchConfig,
    semantic: &dyn SemanticRetriever,
    keyword: &dyn KeywordRetriever,
    embedder: Option<&dyn Embedder>,
) -> Result<Vec<HybridResult>> {
    let query = query.trim();
    if query.is_empty() || config.top_k == 0 {
        return Ok(Vec::new());
    }

    match config.mode {
        SearchMode::Semantic => {
            let embedder = required_embedder(embedder)?;
            let hits = semantic_hits(
                query,
                config.top_k,
                config.filter.as_deref(),
                semantic,
                embedder,
            )
            .await?;
            Ok(hits
                .into_iter()
                .take(config.top_k)
                .map(semantic_result)
                .collect())
        }
        SearchMode::Keyword => {
            let hits = keyword_hits(query, config.top_k, keyword)?;
            let hydrated = hydrate_keyword_content(&hits, semantic).await?;
            Ok(hits
                .into_iter()
                .take(config.top_k)
                .map(|hit| {
                    let hydrated = hydrated.get(hit.chunk_id.as_str());
                    keyword_result(hit, hydrated)
                })
                .collect())
        }
        SearchMode::Hybrid => {
            let embedder = required_embedder(embedder)?;
            let candidate_limit = config.candidate_limit();
            let semantic_future = semantic_hits(
                query,
                candidate_limit,
                config.filter.as_deref(),
                semantic,
                embedder,
            );
            let keyword_future = async { keyword_hits(query, candidate_limit, keyword) };
            let (semantic_hits, keyword_hits) = tokio::join!(semantic_future, keyword_future);
            let semantic_hits = semantic_hits?;
            let keyword_hits = keyword_hits?;

            let weights = AdaptiveWeights::compute(query);
            let hydrated =
                hydrate_missing_keyword_content(&semantic_hits, &keyword_hits, semantic).await?;
            Ok(
                fuse_hits_with_weights(&semantic_hits, &keyword_hits, &hydrated, weights)
                    .into_iter()
                    .take(config.top_k)
                    .collect(),
            )
        }
    }
}

fn required_embedder(embedder: Option<&dyn Embedder>) -> Result<&dyn Embedder> {
    embedder.ok_or_else(|| {
        VektorError::Config("semantic or hybrid search requires an embedder".to_string())
    })
}

async fn semantic_hits(
    query: &str,
    top_k: usize,
    filter: Option<&str>,
    semantic: &dyn SemanticRetriever,
    embedder: &dyn Embedder,
) -> Result<Vec<VectorSearchResult>> {
    let query_vec = embedder.embed_query(query).await?;
    semantic.search_semantic(&query_vec, top_k, filter).await
}

async fn hydrate_keyword_content(
    keyword_hits: &[KeywordHit],
    semantic: &dyn SemanticRetriever,
) -> Result<HashMap<String, VectorSearchResult>> {
    let chunk_ids = keyword_hits
        .iter()
        .map(|hit| hit.chunk_id.clone())
        .collect::<Vec<_>>();
    semantic.hydrate_chunks_by_id(&chunk_ids).await
}

async fn hydrate_missing_keyword_content(
    semantic_hits: &[VectorSearchResult],
    keyword_hits: &[KeywordHit],
    semantic: &dyn SemanticRetriever,
) -> Result<HashMap<String, VectorSearchResult>> {
    let semantic_by_id = semantic_hits
        .iter()
        .map(|hit| hit.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let chunk_ids = keyword_hits
        .iter()
        .filter(|hit| !semantic_by_id.contains(hit.chunk_id.as_str()))
        .map(|hit| hit.chunk_id.clone())
        .collect::<Vec<_>>();
    semantic.hydrate_chunks_by_id(&chunk_ids).await
}

fn keyword_hits(
    query: &str,
    top_k: usize,
    keyword: &dyn KeywordRetriever,
) -> Result<Vec<KeywordHit>> {
    let expanded_query = SynonymExpander::expand(query);
    keyword.search_keyword(&expanded_query, top_k)
}

fn fuse_hits_with_weights(
    semantic_hits: &[VectorSearchResult],
    keyword_hits: &[KeywordHit],
    hydrated_keyword_hits: &HashMap<String, VectorSearchResult>,
    weights: AdaptiveWeights,
) -> Vec<HybridResult> {
    let semantic_ranked = semantic_hits
        .iter()
        .map(|hit| RankedId::new(hit.id.clone()))
        .collect::<Vec<_>>();
    let keyword_ranked = keyword_hits
        .iter()
        .map(|hit| RankedId::new(hit.chunk_id.clone()))
        .collect::<Vec<_>>();
    let semantic_by_id = semantic_hits
        .iter()
        .map(|hit| (hit.id.as_str(), hit))
        .collect::<HashMap<_, _>>();
    let keyword_by_id = keyword_hits
        .iter()
        .map(|hit| (hit.chunk_id.as_str(), hit))
        .collect::<HashMap<_, _>>();

    rrf_fuse(&semantic_ranked, &keyword_ranked, RRF_K, weights)
        .into_iter()
        .filter_map(|hit| {
            let semantic = semantic_by_id.get(hit.chunk_id.as_str()).copied();
            let keyword = keyword_by_id.get(hit.chunk_id.as_str()).copied();
            let hydrated = hydrated_keyword_hits.get(hit.chunk_id.as_str());
            merged_result(&hit.chunk_id, hit.score, semantic, keyword, hydrated)
        })
        .collect()
}

fn semantic_result(hit: VectorSearchResult) -> HybridResult {
    let relevance_score = semantic_relevance(hit.score);
    HybridResult {
        chunk_id: hit.id,
        rel_path: hit.rel_path,
        start_line: u64::from(hit.start_line),
        end_line: u64::from(hit.end_line),
        symbol_name: hit.symbol_name,
        symbol_type: hit.symbol_type,
        language: hit.language,
        content: hit.content,
        relevance_score,
        semantic_score: Some(hit.score),
        keyword_score: None,
        last_modified: hit.last_modified,
    }
}

fn keyword_result(hit: KeywordHit, hydrated: Option<&VectorSearchResult>) -> HybridResult {
    let content = hydrated
        .map(|hit| hit.content.clone())
        .unwrap_or_else(|| hit.content.clone());
    let symbol_type = hydrated.and_then(|hit| hit.symbol_type.clone());
    let last_modified = hydrated.map(|hit| hit.last_modified).unwrap_or(0);

    HybridResult {
        chunk_id: hit.chunk_id,
        rel_path: hit.rel_path,
        start_line: hit.start_line,
        end_line: hit.end_line,
        symbol_name: hit.symbol_name,
        symbol_type,
        language: hit.language,
        content,
        relevance_score: hit.score,
        semantic_score: None,
        keyword_score: Some(hit.score),
        last_modified,
    }
}

fn merged_result(
    chunk_id: &str,
    relevance_score: f32,
    semantic: Option<&VectorSearchResult>,
    keyword: Option<&KeywordHit>,
    hydrated: Option<&VectorSearchResult>,
) -> Option<HybridResult> {
    if let Some(hit) = semantic {
        return Some(HybridResult {
            chunk_id: hit.id.clone(),
            rel_path: hit.rel_path.clone(),
            start_line: u64::from(hit.start_line),
            end_line: u64::from(hit.end_line),
            symbol_name: hit.symbol_name.clone(),
            symbol_type: hit.symbol_type.clone(),
            language: hit.language.clone(),
            content: hit.content.clone(),
            relevance_score,
            semantic_score: Some(hit.score),
            keyword_score: keyword.map(|hit| hit.score),
            last_modified: hit.last_modified,
        });
    }

    if let Some(hit) = hydrated {
        return Some(HybridResult {
            chunk_id: hit.id.clone(),
            rel_path: hit.rel_path.clone(),
            start_line: u64::from(hit.start_line),
            end_line: u64::from(hit.end_line),
            symbol_name: hit.symbol_name.clone(),
            symbol_type: hit.symbol_type.clone(),
            language: hit.language.clone(),
            content: hit.content.clone(),
            relevance_score,
            semantic_score: None,
            keyword_score: keyword.map(|hit| hit.score),
            last_modified: hit.last_modified,
        });
    }

    keyword.map(|hit| HybridResult {
        chunk_id: chunk_id.to_string(),
        rel_path: hit.rel_path.clone(),
        start_line: hit.start_line,
        end_line: hit.end_line,
        symbol_name: hit.symbol_name.clone(),
        symbol_type: None,
        language: hit.language.clone(),
        content: String::new(),
        relevance_score,
        semantic_score: None,
        keyword_score: Some(hit.score),
        last_modified: 0,
    })
}

fn semantic_relevance(distance: f32) -> f32 {
    let distance = if distance.is_finite() {
        distance.max(0.0)
    } else {
        f32::MAX
    };
    1.0 / (1.0 + distance)
}

#[cfg(test)]
pub mod tests {
    use std::{
        collections::HashMap,
        path::Path,
        sync::Mutex,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{
        chunker::{Chunk, Language},
        config::{Config, IndexConfig},
        state::hash_content,
    };

    struct FakeEmbedder {
        dim: usize,
        vectors: HashMap<String, Vec<f32>>,
        received: Mutex<Vec<String>>,
    }

    impl FakeEmbedder {
        fn new(vectors: impl IntoIterator<Item = (&'static str, Vec<f32>)>) -> Self {
            Self {
                dim: 2,
                vectors: vectors
                    .into_iter()
                    .map(|(query, vector)| (query.to_string(), vector))
                    .collect(),
                received: Mutex::new(Vec::new()),
            }
        }

        fn received_texts(&self) -> Vec<String> {
            self.received.lock().expect("received lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.received
                .lock()
                .expect("received lock")
                .extend_from_slice(texts);

            texts
                .iter()
                .map(|text| {
                    self.vectors.get(text).cloned().ok_or_else(|| {
                        VektorError::Embedding(format!("missing fake vector for `{text}`"))
                    })
                })
                .collect()
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn name(&self) -> &str {
            "fake-search-embedder"
        }

        fn prefix_for_document(&self) -> &str {
            "doc: "
        }

        fn prefix_for_query(&self) -> &str {
            "query: "
        }
    }

    struct FakeSemanticRetriever {
        hits: Vec<VectorSearchResult>,
        hydration: HashMap<String, VectorSearchResult>,
    }

    impl FakeSemanticRetriever {
        fn new(hits: Vec<VectorSearchResult>) -> Self {
            let hydration = hits
                .iter()
                .cloned()
                .map(|hit| (hit.id.clone(), hit))
                .collect();
            Self { hits, hydration }
        }

        fn with_hydration(
            hits: Vec<VectorSearchResult>,
            hydration: Vec<VectorSearchResult>,
        ) -> Self {
            Self {
                hits,
                hydration: hydration
                    .into_iter()
                    .map(|hit| (hit.id.clone(), hit))
                    .collect(),
            }
        }
    }

    #[async_trait::async_trait]
    impl SemanticRetriever for FakeSemanticRetriever {
        async fn search_semantic(
            &self,
            _query_vec: &[f32],
            top_k: usize,
            _filter: Option<&str>,
        ) -> Result<Vec<VectorSearchResult>> {
            Ok(self.hits.iter().take(top_k).cloned().collect())
        }

        async fn hydrate_chunks_by_id(
            &self,
            chunk_ids: &[String],
        ) -> Result<HashMap<String, VectorSearchResult>> {
            Ok(chunk_ids
                .iter()
                .filter_map(|id| self.hydration.get(id).cloned().map(|hit| (id.clone(), hit)))
                .collect())
        }
    }

    struct FakeKeywordRetriever {
        hits: Vec<KeywordHit>,
        queries: Mutex<Vec<(String, usize)>>,
    }

    impl FakeKeywordRetriever {
        fn new(hits: Vec<KeywordHit>) -> Self {
            Self {
                hits,
                queries: Mutex::new(Vec::new()),
            }
        }

        fn queries(&self) -> Vec<(String, usize)> {
            self.queries.lock().expect("queries lock").clone()
        }
    }

    impl KeywordRetriever for FakeKeywordRetriever {
        fn search_keyword(&self, query: &str, top_k: usize) -> Result<Vec<KeywordHit>> {
            self.queries
                .lock()
                .expect("queries lock")
                .push((query.to_string(), top_k));
            Ok(self.hits.iter().take(top_k).cloned().collect())
        }
    }

    fn vector_hit(id: &str, distance: f32) -> VectorSearchResult {
        VectorSearchResult {
            score: distance,
            id: id.to_string(),
            content_hash: format!("hash-{id}"),
            rel_path: format!("src/{id}.rs"),
            start_line: 1,
            end_line: 5,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("content for {id}"),
            last_modified: 1_700_000_000,
        }
    }

    fn keyword_hit(id: &str, score: f32) -> KeywordHit {
        KeywordHit {
            chunk_id: id.to_string(),
            rel_path: format!("src/{id}.rs"),
            content: format!("keyword content for {id}"),
            score,
            start_line: 1,
            end_line: 5,
            symbol_name: Some(format!("symbol_{id}")),
            language: "rust".to_string(),
        }
    }

    fn ids(results: &[HybridResult]) -> Vec<&str> {
        results
            .iter()
            .map(|result| result.chunk_id.as_str())
            .collect()
    }

    fn config(mode: SearchMode, top_k: usize) -> HybridSearchConfig {
        HybridSearchConfig::new(mode, top_k)
    }

    #[tokio::test]
    async fn hybrid_differs_from_single_mode() {
        let semantic = FakeSemanticRetriever::new(vec![
            vector_hit("sem-only", 0.0),
            vector_hit("shared", 0.1),
            vector_hit("kw-only", 3.0),
        ]);
        let keyword = FakeKeywordRetriever::new(vec![
            keyword_hit("kw-only", 10.0),
            keyword_hit("shared", 6.0),
        ]);
        let embedder = FakeEmbedder::new([("query: auth", vec![1.0, 0.0])]);

        let hybrid = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Hybrid, 3),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("hybrid search");
        let semantic_only = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Semantic, 3),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("semantic search");
        let keyword_only = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Keyword, 3),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("keyword search");

        assert_eq!(ids(&hybrid), ["shared", "kw-only", "sem-only"]);
        assert_ne!(ids(&hybrid), ids(&semantic_only));
        assert_ne!(ids(&hybrid), ids(&keyword_only));
        assert_eq!(embedder.received_texts()[0], "query: auth");
        assert!(
            keyword.queries()[0].0.contains("authentication"),
            "BM25 leg receives synonym-expanded query"
        );
    }

    #[tokio::test]
    async fn adaptive_weights_shift_ranking() {
        let semantic = FakeSemanticRetriever::new(vec![
            vector_hit("semantic-auth", 0.0),
            vector_hit("keyword-auth", 0.2),
        ]);
        let keyword = FakeKeywordRetriever::new(vec![
            keyword_hit("keyword-auth", 10.0),
            keyword_hit("semantic-auth", 5.0),
        ]);
        let embedder = FakeEmbedder::new([
            ("query: validate_token AuthMiddleware", vec![1.0, 0.0]),
            ("query: how does authentication work", vec![1.0, 0.0]),
        ]);

        let identifier_heavy = search_hybrid_with_retrievers(
            "validate_token AuthMiddleware",
            &config(SearchMode::Hybrid, 2),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("identifier search");
        let natural_language = search_hybrid_with_retrievers(
            "how does authentication work",
            &config(SearchMode::Hybrid, 2),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("natural-language search");

        assert_eq!(identifier_heavy[0].chunk_id, "keyword-auth");
        assert_eq!(natural_language[0].chunk_id, "semantic-auth");
    }

    #[tokio::test]
    async fn semantic_mode_reports_high_is_better_relevance() {
        let semantic =
            FakeSemanticRetriever::new(vec![vector_hit("near", 0.0), vector_hit("far", 3.0)]);
        let keyword = FakeKeywordRetriever::new(Vec::new());
        let embedder = FakeEmbedder::new([("query: explain auth", vec![1.0, 0.0])]);

        let results = search_hybrid_with_retrievers(
            "explain auth",
            &config(SearchMode::Semantic, 2),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("semantic search");

        assert_eq!(ids(&results), ["near", "far"]);
        assert_eq!(results[0].semantic_score, Some(0.0));
        assert_eq!(results[1].semantic_score, Some(3.0));
        assert!(results[0].relevance_score > results[1].relevance_score);
        assert!((results[0].relevance_score - 1.0).abs() < f32::EPSILON);
        assert!((results[1].relevance_score - 0.25).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn synonym_expansion_recall() {
        let fixture = TextIndexFixture::new().add_and_commit(&[chunk(
            "auth-doc",
            "src/auth.rs",
            "authentication middleware verifies the session",
            Some("authenticate"),
        )]);
        let semantic =
            FakeSemanticRetriever::with_hydration(Vec::new(), vec![vector_hit("auth-doc", 0.0)]);
        let embedder = FakeEmbedder::new([]);

        let results = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Keyword, 5),
            &semantic,
            &fixture.index,
            &embedder,
        )
        .await
        .expect("keyword search");

        assert_eq!(ids(&results), ["auth-doc"]);
        assert_eq!(results[0].content, "content for auth-doc");
        assert!(
            embedder.received_texts().is_empty(),
            "keyword mode must not call semantic embedding"
        );
    }

    #[tokio::test]
    async fn keyword_only_results_are_hydrated_with_content() {
        let semantic =
            FakeSemanticRetriever::with_hydration(Vec::new(), vec![vector_hit("kw-only", 0.0)]);
        let keyword = FakeKeywordRetriever::new(vec![keyword_hit("kw-only", 9.0)]);
        let embedder = FakeEmbedder::new([]);

        let results = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Keyword, 1),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("keyword search");

        assert_eq!(ids(&results), ["kw-only"]);
        assert_eq!(results[0].content, "content for kw-only");
        assert_eq!(results[0].keyword_score, Some(9.0));
        assert_eq!(results[0].semantic_score, None);
    }

    #[tokio::test]
    async fn hybrid_keyword_only_fallbacks_are_hydrated_with_content() {
        let semantic = FakeSemanticRetriever::with_hydration(
            vec![vector_hit("sem-only", 0.0)],
            vec![vector_hit("kw-only", 0.0)],
        );
        let keyword = FakeKeywordRetriever::new(vec![keyword_hit("kw-only", 9.0)]);
        let embedder =
            FakeEmbedder::new([("query: validate_token AuthMiddleware", vec![1.0, 0.0])]);

        let results = search_hybrid_with_retrievers(
            "validate_token AuthMiddleware",
            &config(SearchMode::Hybrid, 2),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("hybrid search");
        let kw_only = results
            .iter()
            .find(|result| result.chunk_id == "kw-only")
            .expect("keyword-only fallback result");

        assert_eq!(kw_only.content, "content for kw-only");
        assert_eq!(kw_only.keyword_score, Some(9.0));
        assert_eq!(kw_only.semantic_score, None);
    }

    #[tokio::test]
    async fn empty_query_and_top_k_zero_return_empty() {
        let semantic = FakeSemanticRetriever::new(vec![vector_hit("unused", 0.0)]);
        let keyword = FakeKeywordRetriever::new(vec![keyword_hit("unused", 1.0)]);
        let embedder = FakeEmbedder::new([]);

        let empty_query = search_hybrid_with_retrievers(
            " \n\t ",
            &config(SearchMode::Hybrid, 5),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("empty query");
        let top_k_zero = search_hybrid_with_retrievers(
            "auth",
            &config(SearchMode::Hybrid, 0),
            &semantic,
            &keyword,
            &embedder,
        )
        .await
        .expect("top_k zero");

        assert!(empty_query.is_empty());
        assert!(top_k_zero.is_empty());
        assert!(embedder.received_texts().is_empty());
        assert!(keyword.queries().is_empty());
    }

    #[tokio::test]
    #[ignore = "deterministic performance smoke; run before closing Phase 4"]
    async fn ten_k_chunk_latency_p95_under_300ms() {
        let semantic_hits = (0..10_000)
            .map(|i| vector_hit(&format!("chunk-{i:05}"), i as f32))
            .collect::<Vec<_>>();
        let keyword_hits = (0..10_000)
            .rev()
            .map(|i| keyword_hit(&format!("chunk-{i:05}"), i as f32))
            .collect::<Vec<_>>();
        let semantic = FakeSemanticRetriever::new(semantic_hits);
        let keyword = FakeKeywordRetriever::new(keyword_hits);
        let embedder = FakeEmbedder::new([("query: auth", vec![1.0, 0.0])]);
        let mut search_config = config(SearchMode::Hybrid, 20);
        search_config.candidate_multiplier = 500;
        search_config.max_candidates = 10_000;

        search_hybrid_with_retrievers("auth", &search_config, &semantic, &keyword, &embedder)
            .await
            .expect("warmup search");

        let mut durations = Vec::new();
        for _ in 0..8 {
            let start = Instant::now();
            let results = search_hybrid_with_retrievers(
                "auth",
                &search_config,
                &semantic,
                &keyword,
                &embedder,
            )
            .await
            .expect("latency search");
            assert_eq!(results.len(), 20);
            durations.push(start.elapsed());
        }

        durations.sort();
        let p95_index = (durations.len() * 95).div_ceil(100).saturating_sub(1);
        let p95 = durations[p95_index];
        assert!(
            p95 < Duration::from_millis(300),
            "10K fixture P95 {p95:?} exceeded 300ms; samples={durations:?}"
        );
    }

    struct TextIndexFixture {
        _project: tempfile::TempDir,
        _data_dir: tempfile::TempDir,
        index: TextIndex,
    }

    impl TextIndexFixture {
        fn new() -> Self {
            let project = tempfile::tempdir().expect("project");
            let data_dir = tempfile::tempdir().expect("data dir");
            let index = TextIndex::new(project.path(), &config_with_data_dir(data_dir.path()))
                .expect("create text index");

            Self {
                _project: project,
                _data_dir: data_dir,
                index,
            }
        }

        fn add_and_commit(mut self, chunks: &[Chunk]) -> Self {
            self.index.add_chunks(chunks).expect("add chunks");
            self.index.commit().expect("commit chunks");
            self
        }
    }

    fn config_with_data_dir(data_dir: &Path) -> Config {
        Config {
            index: IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn chunk(id: &str, rel_path: &str, content: &str, symbol_name: Option<&str>) -> Chunk {
        Chunk {
            id: id.to_string(),
            content: content.to_string(),
            content_hash: hash_content(content),
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 5,
            symbol_name: symbol_name.map(ToOwned::to_owned),
            symbol_type: Some("function".to_string()),
            language: Some(Language::Rust),
        }
    }
}
