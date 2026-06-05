use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    context::{
        budget::TokenCounter,
        dedup::Deduplicator,
        expander::{RelatedChunkStore, RelatedExpander},
        recency::RecencyTracker,
        types::{
            AssemblyConfig, ChunkSource, Confidence, ContextChunk, ContextPackage, GapReason,
            ResultCluster, SearchMetadata,
        },
    },
    error::Result,
    search::hybrid::HybridResult,
};

#[allow(dead_code)]
pub(crate) struct RelatedExpansion<'a, S>
where
    S: RelatedChunkStore,
{
    pub(crate) query_vec: &'a [f32],
    pub(crate) store: &'a S,
}

#[allow(dead_code)]
pub(crate) struct ContextAssembler {
    recency: RecencyTracker,
    index_status: String,
    index_coverage_pct: f64,
    cache_hit: bool,
    search_time_ms: u128,
}

#[allow(dead_code)]
impl ContextAssembler {
    pub(crate) fn new() -> Self {
        Self {
            recency: RecencyTracker::now(),
            index_status: "full".to_string(),
            index_coverage_pct: 100.0,
            cache_hit: false,
            search_time_ms: 0,
        }
    }

    pub(crate) fn with_now_and_status(now: i64, index_status: &str) -> Self {
        Self {
            recency: RecencyTracker::at(now),
            index_status: index_status.to_string(),
            index_coverage_pct: if index_status == "full" { 100.0 } else { 0.0 },
            cache_hit: false,
            search_time_ms: 0,
        }
    }

    pub(crate) fn with_status(index_status: &str) -> Self {
        Self {
            recency: RecencyTracker::now(),
            index_status: index_status.to_string(),
            index_coverage_pct: if index_status == "full" { 100.0 } else { 0.0 },
            cache_hit: false,
            search_time_ms: 0,
        }
    }

    pub(crate) fn with_metadata(
        mut self,
        search_time_ms: u128,
        index_coverage_pct: f64,
        cache_hit: bool,
    ) -> Self {
        self.search_time_ms = search_time_ms;
        self.index_coverage_pct = index_coverage_pct;
        self.cache_hit = cache_hit;
        self
    }

    pub(crate) async fn assemble<S>(
        &self,
        results: Vec<HybridResult>,
        config: &AssemblyConfig,
        related: Option<RelatedExpansion<'_, S>>,
    ) -> Result<ContextPackage>
    where
        S: RelatedChunkStore,
    {
        let direct_results_count = results.len();
        let max_raw_score = max_hybrid_score(&results);
        let mut threshold_filtered = false;
        let mut direct = Vec::new();

        for mut result in results {
            if !config.include_docs && is_doc_path(&result.rel_path) {
                continue;
            }
            if config
                .scope
                .as_deref()
                .is_some_and(|scope| !path_in_scope(&result.rel_path, scope))
            {
                continue;
            }

            result.relevance_score = normalized_score(result.relevance_score, max_raw_score);
            if result.relevance_score < config.min_relevance {
                threshold_filtered = true;
                continue;
            }
            direct.push(result);
        }

        let before_dedup = direct.len();
        if config.deduplicate {
            direct = Deduplicator::deduplicate(direct);
        }
        let chunks_deduplicated = before_dedup.saturating_sub(direct.len());

        let mut chunks = direct
            .iter()
            .cloned()
            .map(context_chunk_from_hybrid)
            .collect::<Vec<_>>();

        let missing_related_context = config.include_related && related.is_none();
        if config.include_related
            && let Some(related) = &related
        {
            let expanded = RelatedExpander::new()
                .expand(&direct, related.query_vec, related.store)
                .await?;
            chunks.extend(expanded);
        }

        for chunk in &mut chunks {
            chunk.relevance_score = self
                .recency
                .score(chunk.relevance_score, chunk.last_modified);
        }
        chunks.sort_by(|left, right| {
            right
                .relevance_score
                .total_cmp(&left.relevance_score)
                .then_with(|| left.rel_path.cmp(&right.rel_path))
                .then_with(|| left.lines.cmp(&right.lines))
        });

        let allocated = allocate_budget(chunks, config)?;
        let expanded_results = allocated.iter().filter(|chunk| chunk.is_expanded).count();
        let total_tokens = TokenCounter::count_exact(&package_text(&allocated))?;
        let files_included = files_included(&allocated);
        let budget_used_pct = percentage(total_tokens, config.token_budget);
        let budget_gap_reason =
            budget_gap_reason(&self.index_status, threshold_filtered, total_tokens, config);
        let result_confidence = confidence(&allocated, config.min_relevance);
        let clusters = clusters(&allocated);
        let missing_context_warnings = warnings(
            &self.index_status,
            chunks_deduplicated,
            config.include_related,
            missing_related_context,
        );
        let suggested_action = suggested_action(result_confidence, &self.index_status, &allocated);

        Ok(ContextPackage {
            chunks: allocated,
            files_included,
            total_tokens,
            budget_used_pct,
            missing_context_warnings,
            search_metadata: SearchMetadata {
                search_time_ms: self.search_time_ms,
                direct_results: direct_results_count,
                expanded_results,
            },
            result_confidence,
            budget_gap_reason,
            suggested_action,
            clusters,
            chunks_deduplicated,
            index_status: self.index_status.clone(),
            index_coverage_pct: self.index_coverage_pct,
            cache_hit: self.cache_hit,
        })
    }
}

fn context_chunk_from_hybrid(result: HybridResult) -> ContextChunk {
    ContextChunk {
        chunk_id: result.chunk_id,
        content: result.content,
        rel_path: result.rel_path,
        lines: (result.start_line, result.end_line),
        symbol: result.symbol_name,
        symbol_type: result.symbol_type,
        language: result.language,
        relevance_score: result.relevance_score,
        source: ChunkSource::Search,
        reason: "Primary search result for the query".to_string(),
        is_expanded: false,
        last_modified: result.last_modified,
    }
}

fn allocate_budget(
    chunks: Vec<ContextChunk>,
    config: &AssemblyConfig,
) -> Result<Vec<ContextChunk>> {
    if chunks.is_empty() || config.token_budget == 0 {
        return Ok(Vec::new());
    }

    let heuristic_limit = config.token_budget.saturating_mul(90).div_ceil(100).max(1);
    let mut selected = Vec::new();
    let mut remaining = Vec::new();
    let mut estimate_used = 0usize;
    let mut files = HashSet::new();

    for chunk in chunks {
        if !files.contains(chunk.rel_path.as_str()) && files.len() >= config.max_files {
            remaining.push(chunk);
            continue;
        }

        let estimate = TokenCounter::estimate(&chunk.content, &chunk.language);
        if !selected.is_empty() && estimate_used.saturating_add(estimate) > heuristic_limit {
            remaining.push(chunk);
            continue;
        }

        estimate_used = estimate_used.saturating_add(estimate);
        files.insert(chunk.rel_path.clone());
        selected.push(chunk);
    }

    if selected.is_empty() && !remaining.is_empty() {
        let first = remaining.remove(0);
        files.insert(first.rel_path.clone());
        selected.push(first);
    }

    let mut exact = TokenCounter::count_exact(&package_text(&selected))?;
    let refill_floor = config.token_budget.saturating_mul(95).div_ceil(100);
    while exact < refill_floor && !remaining.is_empty() {
        let next = remaining.remove(0);
        if !files.contains(next.rel_path.as_str()) && files.len() >= config.max_files {
            continue;
        }
        let mut candidate = selected.clone();
        candidate.push(next.clone());
        let candidate_exact = TokenCounter::count_exact(&package_text(&candidate))?;
        if candidate_exact > config.token_budget {
            continue;
        }
        files.insert(next.rel_path.clone());
        selected = candidate;
        exact = candidate_exact;
    }

    truncate_to_budget(&mut selected, config.token_budget)?;
    Ok(selected)
}

fn truncate_to_budget(chunks: &mut Vec<ContextChunk>, token_budget: usize) -> Result<()> {
    loop {
        let total = TokenCounter::count_exact(&package_text(chunks))?;
        if total <= token_budget || chunks.is_empty() {
            return Ok(());
        }

        let last_index = chunks.len() - 1;
        let original = chunks[last_index].content.clone();
        let char_count = original.chars().count();
        if char_count == 0 {
            chunks.pop();
            continue;
        }

        let mut low = 0usize;
        let mut high = char_count;
        while low < high {
            let mid = (low + high).div_ceil(2);
            chunks[last_index].content = original.chars().take(mid).collect();
            let candidate_total = TokenCounter::count_exact(&package_text(chunks))?;
            if candidate_total <= token_budget {
                low = mid;
            } else {
                high = mid.saturating_sub(1);
            }
        }

        if low == 0 {
            chunks.pop();
        } else {
            chunks[last_index].content = original.chars().take(low).collect();
        }
    }
}

fn package_text(chunks: &[ContextChunk]) -> String {
    chunks
        .iter()
        .map(|chunk| chunk.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn max_hybrid_score(results: &[HybridResult]) -> f32 {
    results
        .iter()
        .map(|result| result.relevance_score)
        .fold(0.0, f32::max)
}

fn normalized_score(score: f32, max_score: f32) -> f32 {
    if max_score <= 0.0 {
        return 0.0;
    }
    (score / max_score).clamp(0.0, 1.0)
}

fn is_doc_path(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    if lower.starts_with("docs/") || lower.contains("/docs/") {
        return true;
    }
    matches!(
        Path::new(&lower)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("adoc" | "asciidoc" | "md" | "markdown" | "rst" | "txt")
    )
}

fn path_in_scope(rel_path: &str, scope: &str) -> bool {
    rel_path == scope
        || rel_path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn files_included(chunks: &[ContextChunk]) -> Vec<String> {
    let mut files = chunks
        .iter()
        .map(|chunk| chunk.rel_path.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 1000.0).round() / 10.0
}

fn budget_gap_reason(
    index_status: &str,
    threshold_filtered: bool,
    total_tokens: usize,
    config: &AssemblyConfig,
) -> Option<GapReason> {
    if index_status != "full" {
        return Some(GapReason::IndexIncomplete);
    }
    if threshold_filtered {
        return Some(GapReason::ThresholdFiltered);
    }
    if total_tokens < config.token_budget {
        return Some(GapReason::NoMoreRelevant);
    }
    None
}

fn confidence(chunks: &[ContextChunk], min_relevance: f32) -> Confidence {
    let Some(top_score) = chunks.first().map(|chunk| chunk.relevance_score) else {
        return Confidence::Low;
    };
    let above_threshold = chunks
        .iter()
        .filter(|chunk| chunk.relevance_score >= min_relevance)
        .count();

    if top_score < 0.5 || above_threshold < 2 {
        Confidence::Low
    } else if top_score > 0.8 && above_threshold >= 3 {
        Confidence::High
    } else {
        Confidence::Medium
    }
}

fn clusters(chunks: &[ContextChunk]) -> Vec<ResultCluster> {
    let mut groups: HashMap<String, Vec<f32>> = HashMap::new();
    for chunk in chunks {
        groups
            .entry(path_prefix(&chunk.rel_path))
            .or_default()
            .push(chunk.relevance_score);
    }
    if groups.len() <= 2 {
        return Vec::new();
    }

    let mut clusters = groups
        .into_iter()
        .map(|(path_prefix, scores)| {
            let chunk_count = scores.len();
            let avg_relevance = scores.iter().sum::<f32>() / chunk_count as f32;
            ResultCluster {
                path_prefix,
                chunk_count,
                avg_relevance,
            }
        })
        .collect::<Vec<_>>();
    clusters.sort_by(|left, right| left.path_prefix.cmp(&right.path_prefix));
    clusters
}

fn path_prefix(rel_path: &str) -> String {
    Path::new(rel_path)
        .parent()
        .map(|parent| parent.to_string_lossy().replace('\\', "/"))
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

fn warnings(
    index_status: &str,
    chunks_deduplicated: usize,
    include_related: bool,
    missing_related_context: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if index_status != "full" {
        warnings.push(format!(
            "index_status is `{index_status}`; results may be incomplete"
        ));
    }
    if chunks_deduplicated > 0 {
        warnings.push(format!(
            "deduplicated {chunks_deduplicated} overlapping context chunks"
        ));
    }
    if include_related && missing_related_context {
        warnings.push("related expansion requested without a related-search store".to_string());
    }
    warnings
}

fn suggested_action(
    confidence: Confidence,
    index_status: &str,
    chunks: &[ContextChunk],
) -> Option<String> {
    if index_status != "full" {
        return Some(
            "Run index_codebase to refresh the project index before relying on this context"
                .to_string(),
        );
    }
    if chunks.is_empty() {
        return Some("Broaden the query, lower min_relevance, or adjust scope".to_string());
    }
    if confidence == Confidence::Low {
        return Some("Try a broader query or include more files".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::{
            expander::RelatedChunkStore,
            types::{AssemblyConfig, Confidence, GapReason},
        },
        search::hybrid::HybridResult,
        vector_store::SearchResult,
    };
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeRelatedStore {
        by_path: HashMap<String, Vec<SearchResult>>,
    }

    impl FakeRelatedStore {
        fn with_hits(mut self, rel_path: &str, hits: Vec<SearchResult>) -> Self {
            self.by_path.insert(rel_path.to_string(), hits);
            self
        }
    }

    #[async_trait::async_trait]
    impl RelatedChunkStore for FakeRelatedStore {
        async fn search_related_chunks(
            &self,
            _query_vec: &[f32],
            _top_k: usize,
            filter: Option<&str>,
        ) -> crate::error::Result<Vec<SearchResult>> {
            let rel_path = filter
                .and_then(|filter| filter.strip_prefix("rel_path = '"))
                .and_then(|rest| rest.strip_suffix('\''))
                .unwrap_or_default();
            Ok(self.by_path.get(rel_path).cloned().unwrap_or_default())
        }
    }

    fn config() -> AssemblyConfig {
        AssemblyConfig {
            token_budget: 8_000,
            max_files: 10,
            include_related: false,
            min_relevance: 0.5,
            deduplicate: true,
            include_docs: true,
            scope: None,
        }
    }

    fn hit(id: &str, rel_path: &str, start: u64, end: u64, score: f32) -> HybridResult {
        HybridResult {
            chunk_id: id.to_string(),
            rel_path: rel_path.to_string(),
            start_line: start,
            end_line: end,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("fn {id}() {{}}\n"),
            relevance_score: score,
            semantic_score: Some(score),
            keyword_score: None,
            last_modified: 1_700_000_000,
        }
    }

    fn vector_hit(id: &str, rel_path: &str, distance: f32) -> SearchResult {
        SearchResult {
            score: distance,
            id: id.to_string(),
            content_hash: format!("hash-{id}"),
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 5,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "python".to_string(),
            content: format!("related content {id}"),
            last_modified: 1_700_000_000,
        }
    }

    #[tokio::test]
    async fn assemble_pipeline_order() {
        let assembler = ContextAssembler::with_now_and_status(1_700_000_100, "full");
        let mut config = config();
        config.min_relevance = 0.5;
        let results = vec![
            hit("best", "src/lib.rs", 1, 10, 10.0),
            hit("overlap", "src/lib.rs", 5, 12, 9.0),
            hit("filtered", "src/low.rs", 1, 5, 1.0),
        ];

        let package = assembler
            .assemble::<FakeRelatedStore>(results, &config, None)
            .await
            .expect("assemble");

        assert_eq!(package.chunks.len(), 1);
        assert_eq!(package.chunks[0].chunk_id, "best");
        assert_eq!(package.chunks_deduplicated, 1);
        assert_eq!(
            package.budget_gap_reason,
            Some(GapReason::ThresholdFiltered)
        );
    }

    #[tokio::test]
    async fn related_expansion_exempt_from_min_relevance() {
        let store = FakeRelatedStore::default().with_hits(
            "tests/test_auth.py",
            vec![vector_hit("auth-test", "tests/test_auth.py", 0.25)],
        );
        let assembler = ContextAssembler::with_now_and_status(1_700_000_100, "full");
        let mut config = config();
        config.include_related = true;
        config.min_relevance = 0.9;

        let package = assembler
            .assemble(
                vec![hit("jwt", "src/auth/jwt.py", 1, 5, 10.0)],
                &config,
                Some(RelatedExpansion {
                    query_vec: &[1.0, 0.0],
                    store: &store,
                }),
            )
            .await
            .expect("assemble");

        assert!(
            package
                .chunks
                .iter()
                .any(|chunk| chunk.rel_path == "tests/test_auth.py")
        );
    }

    #[tokio::test]
    async fn recency_boost_respects_min_score_gate() {
        let assembler = ContextAssembler::with_now_and_status(1_700_000_000, "full");
        let mut recent = hit("recent", "src/recent.rs", 1, 5, 1.0);
        recent.last_modified = 1_699_999_900;
        let mut old = hit("old", "src/old.rs", 1, 5, 1.0);
        old.last_modified = 1_600_000_000;
        let mut irrelevant = hit("irrelevant", "src/irrelevant.rs", 1, 5, 0.2);
        irrelevant.last_modified = 1_699_999_900;
        let mut config = config();
        config.min_relevance = 0.0;

        let package = assembler
            .assemble::<FakeRelatedStore>(vec![old, recent, irrelevant], &config, None)
            .await
            .expect("assemble");

        assert_eq!(package.chunks[0].chunk_id, "recent");
        assert!(
            package
                .chunks
                .iter()
                .position(|chunk| chunk.chunk_id == "irrelevant")
                .expect("irrelevant included")
                > 0
        );
    }

    #[tokio::test]
    async fn confidence_heuristic_high_medium_low() {
        let assembler = ContextAssembler::with_now_and_status(1_700_000_100, "full");
        let high = assembler
            .assemble::<FakeRelatedStore>(
                vec![
                    hit("a", "src/a.rs", 1, 1, 10.0),
                    hit("b", "src/b.rs", 1, 1, 9.0),
                    hit("c", "src/c.rs", 1, 1, 8.0),
                ],
                &config(),
                None,
            )
            .await
            .expect("high");
        let low = assembler
            .assemble::<FakeRelatedStore>(vec![hit("x", "src/x.rs", 1, 1, 1.0)], &config(), None)
            .await
            .expect("low");

        assert_eq!(high.result_confidence, Confidence::High);
        assert_eq!(low.result_confidence, Confidence::Low);
    }

    #[tokio::test]
    async fn budget_gap_reason_variants() {
        let mut config = config();
        config.min_relevance = 0.9;
        let partial = ContextAssembler::with_now_and_status(1_700_000_100, "partial")
            .assemble::<FakeRelatedStore>(vec![hit("a", "src/a.rs", 1, 1, 10.0)], &config, None)
            .await
            .expect("partial");

        assert_eq!(partial.budget_gap_reason, Some(GapReason::IndexIncomplete));
    }

    #[tokio::test]
    async fn clusters_group_by_directory() {
        let assembler = ContextAssembler::with_now_and_status(1_700_000_100, "full");
        let mut config = config();
        config.min_relevance = 0.0;
        let package = assembler
            .assemble::<FakeRelatedStore>(
                vec![
                    hit("auth", "src/auth/jwt.rs", 1, 1, 10.0),
                    hit("billing", "src/billing/invoice.rs", 1, 1, 9.0),
                    hit("ui", "web/ui/button.rs", 1, 1, 8.0),
                ],
                &config,
                None,
            )
            .await
            .expect("clusters");

        assert_eq!(package.clusters.len(), 3);
    }

    #[tokio::test]
    async fn token_budget_is_enforced() {
        let assembler = ContextAssembler::with_now_and_status(1_700_000_100, "full");
        let mut config = config();
        config.token_budget = 40;
        config.min_relevance = 0.0;
        let mut large = hit("large", "src/large.rs", 1, 200, 10.0);
        large.content = "let value = compute_value();\n".repeat(200);

        let package = assembler
            .assemble::<FakeRelatedStore>(vec![large], &config, None)
            .await
            .expect("budget");

        assert!(package.total_tokens <= config.token_budget);
        assert!(package.budget_used_pct <= 100.0);
    }
}
