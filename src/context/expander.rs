use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    context::types::{ChunkSource, ContextChunk},
    error::Result,
    search::hybrid::HybridResult,
    vector_store::{SearchResult, VectorStore},
};

const HIGH_TIER_MULTIPLIER: f32 = 0.6;
const LOW_TIER_MULTIPLIER: f32 = 0.4;
const MIN_CHUNK_SIMILARITY: f32 = 0.3;
const MAX_EXPANDED_FILES: usize = 5;
const MAX_CHUNKS_PER_FILE: usize = 3;
const HUB_IMPORT_DEGREE_THRESHOLD: usize = 20;

#[allow(dead_code)]
pub(crate) struct RelatedExpander;

#[allow(dead_code)]
impl RelatedExpander {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn expand<S>(
        &self,
        results: &[HybridResult],
        query_vec: &[f32],
        store: &S,
    ) -> Result<Vec<ContextChunk>>
    where
        S: RelatedChunkStore,
    {
        if results.is_empty() {
            return Ok(Vec::new());
        }

        let direct_files = results
            .iter()
            .map(|result| result.rel_path.as_str())
            .collect::<HashSet<_>>();
        let candidates = candidate_files(results, &direct_files, store);
        let mut expanded = Vec::new();
        let mut expanded_files = HashSet::new();

        for candidate in candidates {
            if expanded_files.len() >= MAX_EXPANDED_FILES {
                break;
            }
            let filter = format!(
                "rel_path = '{}'",
                sql_string_literal(candidate.rel_path.as_str())
            );
            let hits = store
                .search_related_chunks(query_vec, MAX_CHUNKS_PER_FILE, Some(&filter))
                .await?;

            for hit in hits.into_iter().take(MAX_CHUNKS_PER_FILE) {
                let similarity = semantic_similarity(hit.score);
                if similarity <= MIN_CHUNK_SIMILARITY {
                    continue;
                }
                if !expanded_files.contains(hit.rel_path.as_str())
                    && expanded_files.len() >= MAX_EXPANDED_FILES
                {
                    break;
                }
                expanded_files.insert(hit.rel_path.clone());
                expanded.push(context_chunk_from_search_result(
                    hit,
                    similarity * candidate.multiplier,
                    candidate.multiplier,
                ));
            }
        }

        Ok(expanded)
    }
}

#[allow(dead_code)]
#[async_trait::async_trait]
pub(crate) trait RelatedChunkStore: Sync {
    async fn search_related_chunks(
        &self,
        query_vec: &[f32],
        top_k: usize,
        filter: Option<&str>,
    ) -> Result<Vec<SearchResult>>;

    fn import_degree(&self, _rel_path: &str) -> usize {
        0
    }
}

#[async_trait::async_trait]
impl RelatedChunkStore for VectorStore {
    async fn search_related_chunks(
        &self,
        query_vec: &[f32],
        top_k: usize,
        filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        self.search(query_vec, top_k, filter).await
    }
}

#[derive(Debug, Clone)]
struct ExpansionCandidate {
    rel_path: String,
    multiplier: f32,
    order: usize,
}

fn candidate_files<S>(
    results: &[HybridResult],
    direct_files: &HashSet<&str>,
    store: &S,
) -> Vec<ExpansionCandidate>
where
    S: RelatedChunkStore,
{
    let mut by_path: HashMap<String, ExpansionCandidate> = HashMap::new();
    let mut next_order = 0usize;

    for result in results {
        for rel_path in high_tier_candidates(&result.rel_path) {
            insert_candidate(
                &mut by_path,
                &mut next_order,
                rel_path,
                HIGH_TIER_MULTIPLIER,
            );
        }
    }
    for result in results {
        for rel_path in low_tier_candidates(&result.rel_path) {
            insert_candidate(&mut by_path, &mut next_order, rel_path, LOW_TIER_MULTIPLIER);
        }
    }

    let mut candidates = by_path.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .multiplier
            .total_cmp(&left.multiplier)
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.rel_path.cmp(&right.rel_path))
    });

    candidates
        .into_iter()
        .filter(|candidate| !direct_files.contains(candidate.rel_path.as_str()))
        .filter(|candidate| store.import_degree(&candidate.rel_path) <= HUB_IMPORT_DEGREE_THRESHOLD)
        .collect()
}

fn insert_candidate(
    by_path: &mut HashMap<String, ExpansionCandidate>,
    next_order: &mut usize,
    rel_path: String,
    multiplier: f32,
) {
    match by_path.get_mut(&rel_path) {
        Some(existing) if multiplier > existing.multiplier => {
            existing.multiplier = multiplier;
        }
        Some(_) => {}
        None => {
            by_path.insert(
                rel_path.clone(),
                ExpansionCandidate {
                    rel_path,
                    multiplier,
                    order: *next_order,
                },
            );
            *next_order = next_order.saturating_add(1);
        }
    }
}

fn high_tier_candidates(rel_path: &str) -> Vec<String> {
    let path = Path::new(rel_path);
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return Vec::new();
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    candidates.push(format!("tests/test_{stem}.{extension}"));
    candidates.push(format!("tests/{stem}_test.{extension}"));

    if matches!(extension, "js" | "jsx" | "ts" | "tsx") {
        candidates.push(format!("tests/{stem}.test.{extension}"));
    }

    if let Some(parent_name) = parent_dir_name(path)
        && !matches!(parent_name, "src" | "tests" | "test")
    {
        candidates.push(format!("tests/test_{parent_name}.{extension}"));
        candidates.push(format!("tests/{parent_name}_test.{extension}"));
        candidates.push(format!("tests/{parent_name}/test_{stem}.{extension}"));
        candidates.push(format!("tests/{parent_name}/{stem}_test.{extension}"));
        if matches!(extension, "js" | "jsx" | "ts" | "tsx") {
            candidates.push(format!("tests/{parent_name}/{stem}.test.{extension}"));
        }
    }

    dedup_paths(candidates)
}

fn low_tier_candidates(rel_path: &str) -> Vec<String> {
    let path = Path::new(rel_path);
    let Some(parent) = path.parent() else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for file_name in ["mod.rs", "index.ts", "index.tsx", "index.js", "index.jsx"] {
        candidates.push(normalized_path(parent.join(file_name)));
    }
    dedup_paths(candidates)
}

fn parent_dir_name(path: &Path) -> Option<&str> {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
}

fn dedup_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

fn normalized_path(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn context_chunk_from_search_result(
    hit: SearchResult,
    relevance_score: f32,
    multiplier: f32,
) -> ContextChunk {
    ContextChunk {
        chunk_id: hit.id,
        content: hit.content,
        rel_path: hit.rel_path,
        lines: (u64::from(hit.start_line), u64::from(hit.end_line)),
        symbol: hit.symbol_name,
        symbol_type: hit.symbol_type,
        language: hit.language,
        relevance_score,
        source: ChunkSource::Related,
        reason: format!("Related context at {multiplier:.1}x expansion tier"),
        is_expanded: true,
        last_modified: hit.last_modified,
    }
}

fn semantic_similarity(distance: f32) -> f32 {
    let distance = if distance.is_finite() {
        distance.max(0.0)
    } else {
        f32::MAX
    };
    1.0 / (1.0 + distance)
}

fn sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{search::hybrid::HybridResult, vector_store::SearchResult};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    type RecordedSearches = Arc<Mutex<Vec<(usize, Option<String>)>>>;

    #[derive(Default)]
    struct FakeRelatedStore {
        by_path: HashMap<String, Vec<SearchResult>>,
        degrees: HashMap<String, usize>,
        searched: RecordedSearches,
    }

    impl FakeRelatedStore {
        fn with_hits(mut self, rel_path: &str, hits: Vec<SearchResult>) -> Self {
            self.by_path.insert(rel_path.to_string(), hits);
            self
        }

        fn with_degree(mut self, rel_path: &str, degree: usize) -> Self {
            self.degrees.insert(rel_path.to_string(), degree);
            self
        }

        fn searches(&self) -> Vec<(usize, Option<String>)> {
            self.searched.lock().expect("searched lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl RelatedChunkStore for FakeRelatedStore {
        async fn search_related_chunks(
            &self,
            _query_vec: &[f32],
            top_k: usize,
            filter: Option<&str>,
        ) -> crate::error::Result<Vec<SearchResult>> {
            self.searched
                .lock()
                .expect("searched lock")
                .push((top_k, filter.map(ToOwned::to_owned)));
            let rel_path = filter
                .and_then(|filter| filter.strip_prefix("rel_path = '"))
                .and_then(|rest| rest.strip_suffix('\''))
                .unwrap_or_default();
            Ok(self.by_path.get(rel_path).cloned().unwrap_or_default())
        }

        fn import_degree(&self, rel_path: &str) -> usize {
            self.degrees.get(rel_path).copied().unwrap_or(0)
        }
    }

    fn direct_hit(id: &str, rel_path: &str) -> HybridResult {
        HybridResult {
            chunk_id: id.to_string(),
            rel_path: rel_path.to_string(),
            start_line: 10,
            end_line: 20,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "python".to_string(),
            content: format!("direct content {id}"),
            relevance_score: 0.9,
            semantic_score: Some(0.9),
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
    async fn test_file_expands_at_high_tier() {
        let store = FakeRelatedStore::default().with_hits(
            "tests/test_auth.py",
            vec![vector_hit("auth-test", "tests/test_auth.py", 0.25)],
        );
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&[direct_hit("jwt", "src/auth/jwt.py")], &[1.0, 0.0], &store)
            .await
            .expect("expand");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].rel_path, "tests/test_auth.py");
        assert_eq!(expanded[0].source, ChunkSource::Related);
        assert!(expanded[0].is_expanded);
        assert!((expanded[0].relevance_score - 0.48).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn reverse_and_sibling_use_low_tier() {
        let store = FakeRelatedStore::default().with_hits(
            "src/lib/index.ts",
            vec![vector_hit("barrel", "src/lib/index.ts", 0.25)],
        );
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&[direct_hit("foo", "src/lib/foo.ts")], &[1.0, 0.0], &store)
            .await
            .expect("expand");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].rel_path, "src/lib/index.ts");
        assert!((expanded[0].relevance_score - 0.32).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn chunk_level_drops_irrelevant_chunks() {
        let store = FakeRelatedStore::default().with_hits(
            "tests/test_auth.py",
            vec![
                vector_hit("near", "tests/test_auth.py", 0.1),
                vector_hit("far", "tests/test_auth.py", 3.0),
            ],
        );
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&[direct_hit("jwt", "src/auth/jwt.py")], &[1.0, 0.0], &store)
            .await
            .expect("expand");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].chunk_id, "near");
    }

    #[tokio::test]
    async fn caps_limit_files_and_chunks() {
        let mut store = FakeRelatedStore::default();
        for index in 0..8 {
            let rel_path = format!("tests/test_mod{index}.py");
            store = store.with_hits(
                &rel_path,
                (0..5)
                    .map(|chunk| vector_hit(&format!("m{index}-{chunk}"), &rel_path, 0.1))
                    .collect(),
            );
        }
        let hits = (0..8)
            .map(|index| direct_hit(&format!("mod{index}"), &format!("src/mod{index}.py")))
            .collect::<Vec<_>>();
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&hits, &[1.0, 0.0], &store)
            .await
            .expect("expand");

        let files = expanded
            .iter()
            .map(|chunk| chunk.rel_path.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(files.len() <= 5);
        for rel_path in files {
            assert!(
                expanded
                    .iter()
                    .filter(|chunk| chunk.rel_path == rel_path)
                    .count()
                    <= 3
            );
        }
    }

    #[tokio::test]
    async fn hub_files_skipped() {
        let store = FakeRelatedStore::default()
            .with_degree("src/lib/index.ts", 21)
            .with_hits(
                "src/lib/index.ts",
                vec![vector_hit("barrel", "src/lib/index.ts", 0.1)],
            );
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&[direct_hit("foo", "src/lib/foo.ts")], &[1.0, 0.0], &store)
            .await
            .expect("expand");

        assert!(expanded.is_empty());
        assert!(store.searches().iter().all(|(_, filter)| {
            filter
                .as_deref()
                .and_then(|filter| filter.strip_prefix("rel_path = '"))
                .and_then(|rest| rest.strip_suffix('\''))
                != Some("src/lib/index.ts")
        }));
    }

    #[tokio::test]
    async fn files_already_present_as_direct_hits_are_not_readded() {
        let store = FakeRelatedStore::default().with_hits(
            "tests/test_auth.py",
            vec![vector_hit("auth-test", "tests/test_auth.py", 0.1)],
        );
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(
                &[
                    direct_hit("jwt", "src/auth/jwt.py"),
                    direct_hit("auth-test-direct", "tests/test_auth.py"),
                ],
                &[1.0, 0.0],
                &store,
            )
            .await
            .expect("expand");

        assert!(expanded.is_empty());
    }

    #[tokio::test]
    async fn empty_input_no_store_calls() {
        let store = FakeRelatedStore::default();
        let expander = RelatedExpander::new();

        let expanded = expander
            .expand(&[], &[1.0, 0.0], &store)
            .await
            .expect("expand");

        assert!(expanded.is_empty());
        assert!(store.searches().is_empty());
    }
}
