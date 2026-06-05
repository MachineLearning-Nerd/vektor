use std::{
    collections::HashSet,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use lru::LruCache;

use crate::search::hybrid::{HybridResult, SearchMode};

const DEFAULT_CACHE_CAPACITY: usize = 100;
const DEFAULT_TTL: Duration = Duration::from_secs(60);

#[allow(dead_code)]
pub(crate) struct QueryCache {
    entries: LruCache<CacheKey, CacheEntry>,
    ttl: Duration,
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl QueryCache {
    pub(crate) fn new() -> Self {
        Self::with_capacity_and_ttl(DEFAULT_CACHE_CAPACITY, DEFAULT_TTL)
    }

    pub(crate) fn with_capacity_and_ttl(capacity: usize, ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            entries: LruCache::new(capacity),
            ttl,
        }
    }

    pub(crate) fn get(
        &mut self,
        query: &str,
        mode: SearchMode,
        project_hash: &str,
    ) -> Option<&Vec<HybridResult>> {
        let key = CacheKey::new(query, mode, project_hash);
        let expired = self
            .entries
            .get(&key)
            .is_some_and(|entry| entry.inserted_at.elapsed() > self.ttl);
        if expired {
            self.entries.pop(&key);
            return None;
        }

        self.entries.get(&key).map(|entry| &entry.payload)
    }

    pub(crate) fn put(
        &mut self,
        query: &str,
        mode: SearchMode,
        project_hash: &str,
        payload: Vec<HybridResult>,
        files_included: HashSet<String>,
    ) {
        self.put_with_instant(
            query,
            mode,
            project_hash,
            payload,
            files_included,
            Instant::now(),
        );
    }

    pub(crate) fn invalidate_file(&mut self, rel_path: &str) {
        let keys = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.files_included.contains(rel_path))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();

        for key in keys {
            self.entries.pop(&key);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    fn put_with_instant(
        &mut self,
        query: &str,
        mode: SearchMode,
        project_hash: &str,
        payload: Vec<HybridResult>,
        files_included: HashSet<String>,
        inserted_at: Instant,
    ) {
        self.entries.put(
            CacheKey::new(query, mode, project_hash),
            CacheEntry {
                payload,
                files_included,
                inserted_at,
            },
        );
    }

    #[cfg(test)]
    fn put_at(
        &mut self,
        query: &str,
        mode: SearchMode,
        project_hash: &str,
        payload: Vec<HybridResult>,
        files_included: HashSet<String>,
        inserted_at: Instant,
    ) {
        self.put_with_instant(
            query,
            mode,
            project_hash,
            payload,
            files_included,
            inserted_at,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    query: String,
    mode: &'static str,
    project_hash: String,
}

impl CacheKey {
    fn new(query: &str, mode: SearchMode, project_hash: &str) -> Self {
        Self {
            query: query.trim().to_string(),
            mode: mode_key(mode),
            project_hash: project_hash.to_string(),
        }
    }
}

struct CacheEntry {
    payload: Vec<HybridResult>,
    files_included: HashSet<String>,
    inserted_at: Instant,
}

fn mode_key(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Hybrid => "hybrid",
        SearchMode::Semantic => "semantic",
        SearchMode::Keyword => "keyword",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{search::hybrid::HybridResult, search::hybrid::SearchMode};
    use std::{
        collections::HashSet,
        time::{Duration, Instant},
    };

    fn result(id: &str, rel_path: &str) -> HybridResult {
        HybridResult {
            chunk_id: id.to_string(),
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 5,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("content {id}"),
            relevance_score: 1.0,
            semantic_score: Some(1.0),
            keyword_score: None,
            last_modified: 1_700_000_000,
        }
    }

    fn files(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn hit_within_ttl() {
        let mut cache = QueryCache::with_capacity_and_ttl(100, Duration::from_secs(60));
        cache.put(
            " auth ",
            SearchMode::Hybrid,
            "project-a",
            vec![result("a", "src/auth.rs")],
            files(&["src/auth.rs"]),
        );

        let hit = cache
            .get("auth", SearchMode::Hybrid, "project-a")
            .expect("cache hit");

        assert_eq!(hit[0].chunk_id, "a");
    }

    #[test]
    fn expired_entry_is_a_miss() {
        let mut cache = QueryCache::with_capacity_and_ttl(100, Duration::from_secs(60));
        cache.put_at(
            "auth",
            SearchMode::Hybrid,
            "project-a",
            vec![result("a", "src/auth.rs")],
            files(&["src/auth.rs"]),
            Instant::now() - Duration::from_secs(61),
        );

        assert!(cache.get("auth", SearchMode::Hybrid, "project-a").is_none());
        assert!(cache.get("auth", SearchMode::Hybrid, "project-a").is_none());
    }

    #[test]
    fn invalidate_file_is_scoped() {
        let mut cache = QueryCache::with_capacity_and_ttl(100, Duration::from_secs(60));
        cache.put(
            "auth",
            SearchMode::Hybrid,
            "project-a",
            vec![result("auth", "src/auth.rs")],
            files(&["src/auth.rs"]),
        );
        cache.put(
            "billing",
            SearchMode::Hybrid,
            "project-a",
            vec![result("billing", "src/billing.rs")],
            files(&["src/billing.rs"]),
        );

        cache.invalidate_file("src/auth.rs");

        assert!(cache.get("auth", SearchMode::Hybrid, "project-a").is_none());
        assert!(
            cache
                .get("billing", SearchMode::Hybrid, "project-a")
                .is_some()
        );
    }

    #[test]
    fn lru_evicts_when_over_capacity() {
        let mut cache = QueryCache::with_capacity_and_ttl(2, Duration::from_secs(60));
        cache.put(
            "a",
            SearchMode::Hybrid,
            "project",
            vec![result("a", "a.rs")],
            files(&["a.rs"]),
        );
        cache.put(
            "b",
            SearchMode::Hybrid,
            "project",
            vec![result("b", "b.rs")],
            files(&["b.rs"]),
        );
        assert!(cache.get("a", SearchMode::Hybrid, "project").is_some());
        cache.put(
            "c",
            SearchMode::Hybrid,
            "project",
            vec![result("c", "c.rs")],
            files(&["c.rs"]),
        );

        assert!(cache.get("b", SearchMode::Hybrid, "project").is_none());
        assert!(cache.get("a", SearchMode::Hybrid, "project").is_some());
        assert!(cache.get("c", SearchMode::Hybrid, "project").is_some());
    }

    #[test]
    fn mode_and_project_partition_keys() {
        let mut cache = QueryCache::with_capacity_and_ttl(100, Duration::from_secs(60));
        cache.put(
            "auth",
            SearchMode::Hybrid,
            "project-a",
            vec![result("hybrid", "a.rs")],
            files(&["a.rs"]),
        );
        cache.put(
            "auth",
            SearchMode::Keyword,
            "project-a",
            vec![result("keyword", "a.rs")],
            files(&["a.rs"]),
        );
        cache.put(
            "auth",
            SearchMode::Hybrid,
            "project-b",
            vec![result("other-project", "a.rs")],
            files(&["a.rs"]),
        );

        assert_eq!(
            cache
                .get("auth", SearchMode::Hybrid, "project-a")
                .expect("hit")[0]
                .chunk_id,
            "hybrid"
        );
        assert_eq!(
            cache
                .get("auth", SearchMode::Keyword, "project-a")
                .expect("hit")[0]
                .chunk_id,
            "keyword"
        );
        assert_eq!(
            cache
                .get("auth", SearchMode::Hybrid, "project-b")
                .expect("hit")[0]
                .chunk_id,
            "other-project"
        );
    }

    #[test]
    fn bypass_cache_repopulates_after_skipped_get() {
        let mut cache = QueryCache::with_capacity_and_ttl(100, Duration::from_secs(60));

        cache.put(
            "auth",
            SearchMode::Hybrid,
            "project-a",
            vec![result("fresh", "src/auth.rs")],
            files(&["src/auth.rs"]),
        );

        let hit = cache
            .get("auth", SearchMode::Hybrid, "project-a")
            .expect("fresh result was stored after bypass");
        assert_eq!(hit[0].chunk_id, "fresh");
    }
}
