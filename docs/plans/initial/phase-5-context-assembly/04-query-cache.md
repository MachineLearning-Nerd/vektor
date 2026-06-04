# Task 5.4 — `QueryCache` — LRU + file-level invalidation + 60s TTL

**Phase**: 5 — Context Assembly
**Task ID**: 5.4
**PRD reference**: Section 5.3 (QueryCache)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: M
**Depends on**: 4.5
**Blocks**: 5.5

## Objective

Cache pre-assembled search candidates during active editing so repeated queries are
near-instant, while preventing stale results after file changes. PRD §5.3 specifies
an LRU keyed on `(query_text, search_mode, project_hash)`, a 60-second TTL, and — the
v2.2 fix — *file-level* invalidation so editing one file only evicts cached queries
that actually touched that file, not the whole project.

To avoid collisions across request knobs, do not store fully assembled context packages.
Cache `search_hybrid` results **before** request-level filtering (`scope`,
`include_related`, `min_relevance`, `max_files`, `include_docs`, and `token_budget`);
the handler/assembler applies those later.

## Inputs (must exist before starting)

- The cached payload type — raw `search_hybrid` output (`Vec<HybridResult>`) plus
  `files_included: HashSet<String>` for file-level invalidation.
- `SearchMode` (`src/search/hybrid.rs`) — part of the cache key.
- A `project_hash` (stable per indexed project) — part of the cache key.
- `lru = "0.18"` — already a dependency (`Cargo.toml`); NOT a new dep.

## Outputs (must exist after completion)

- A `QueryCache` struct wrapping an `LruCache` with a configurable capacity
  (default **100** entries per PRD §5.3) and a fixed **60s** TTL.
- A cache key derived from `(query_text, SearchMode, project_hash)`.
- Each entry stores its payload plus `files_included: HashSet<String>` (the set
  of `rel_path`s the cached results touched) and an `inserted_at: Instant`.
- Methods (exact names may adjust to fit 5.5's call site):
  - `get(&mut self, query: &str, mode: SearchMode, project_hash: &str) -> Option<&Vec<HybridResult>>`
    — returns `None` on miss OR when the entry is older than 60s (treat expired as
    a miss and evict it).
  - `put(&mut self, query: &str, mode: SearchMode, project_hash: &str, payload: Vec<HybridResult>, files_included: HashSet<String>)`.
  - `invalidate_file(&mut self, rel_path: &str)` — evict only entries whose
    `files_included` contains `rel_path` (v2.2 file-level invalidation).
- A `bypass_cache` path: the caller (5.5 / handler) may skip `get` for a forced
  fresh search; `put` still records the fresh result.

## Approach

- Use `lru::LruCache<CacheKey, CacheEntry>` with `NonZeroUsize` capacity.
- `CacheKey`: a struct (or a normalized `String`) of `(query.trim(), mode,
  project_hash)`. Normalize the query (trim; decide on case) so trivially-different
  spellings of the same query hit the same entry — document the chosen rule.
- Cache value is the raw `Vec<HybridResult>` (the handler applies all policy filters).
- TTL: store `inserted_at: Instant` per entry; on `get`, if
  `inserted_at.elapsed() > Duration::from_secs(60)`, `pop` it and return `None`.
- `invalidate_file`: iterate entries, collect keys whose `files_included` set
  contains the path, then `pop` each. `lru` does not expose predicate eviction
  directly, so collect-then-pop. This is O(n) over the cache (≤100 entries) — fine.
- The cache is shared mutable state; the handler layer wraps it
  (`Arc<Mutex<QueryCache>>` or `tokio::sync::Mutex`) — decided at the 5.6 call
  site, not here. Keep `QueryCache` itself `&mut self`-based and lock-free.

## Acceptance criteria

- [ ] A repeated `(query, mode, project_hash)` within 60s returns the cached
      payload (a hit), not a recomputation.
- [ ] Changing `scope` / `max_files` / `include_related` / `min_relevance` /
      `token_budget` / `include_docs` can reuse the same cached hit candidate pool.
- [ ] An entry older than 60s is treated as a miss and evicted on access.
- [ ] `invalidate_file("src/auth.rs")` evicts only entries whose
      `files_included` contains `src/auth.rs`; an unrelated cached query survives.
- [ ] Capacity is bounded (default 100); inserting beyond capacity evicts the
      least-recently-used entry.
- [ ] Differing `mode` or `project_hash` for the same query text are distinct keys
      (no cross-mode / cross-project bleed).
- [ ] `bypass_cache` skips the `get` but a subsequent `put` repopulates.
- [ ] No `unwrap()` outside `#[cfg(test)]` (`NonZeroUsize` capacity built safely).

## Verification

```bash
cargo build
cargo test context::cache::tests::hit_within_ttl
cargo test context::cache::tests::expired_entry_is_a_miss
cargo test context::cache::tests::invalidate_file_is_scoped
cargo test context::cache::tests::lru_evicts_when_over_capacity
cargo test context::cache::tests::mode_and_project_partition_keys
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **`lru` is already a dependency** (`lru = "0.18"` in `Cargo.toml`) — no NEW dep.
  Verify the `LruCache` constructor (`new(NonZeroUsize)`) and `pop`/`get`/`put`
  signatures against 0.18 before writing; the API shifted around the `NonZeroUsize`
  change in earlier versions.
- TTL via `Instant` is monotonic and immune to wall-clock jumps — preferred over
  `SystemTime` for expiry.
- Test the TTL deterministically by injecting `inserted_at` (or a clock) rather
  than sleeping 60s in a unit test.
