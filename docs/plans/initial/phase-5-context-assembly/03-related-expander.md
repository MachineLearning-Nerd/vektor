# Task 5.3 — `RelatedExpander::expand(...)` — chunk-level, tiered, hub-skipping expansion

**Phase**: 5 — Context Assembly
**Task ID**: 5.3
**PRD reference**: Section 5.3 (RelatedExpander)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: L
**Depends on**: 4.5
**Blocks**: 5.5

## Objective

Take the files surfaced by hybrid search (`Vec<HybridResult>` from 4.5) and
expand the context with architecturally related code — but only the *relevant*
chunks of those related files, scored down so they supplement rather than
displace the direct hits. This is what makes `include_related` actually useful;
PRD §5.3 notes the old file-level expansion was the #1 source of token-budget
waste, and the old 0.3× multiplier got everything filtered out.

## Inputs (must exist before starting)

- `HybridResult` (`src/search/hybrid.rs`) — the search hits to expand from,
  carrying `chunk_id`, `rel_path`, `content`, `relevance_score`.
- `VectorStore::search(query_vec, top_k, filter)` (3.8 / 4.5) — an async call used to
  score a single file's chunks against the query embedding; pass a `filter` of
  `rel_path = '<file>'` to scope the ANN query to one file.
- The query embedding (the same `Vec<f32>` 4.5 produced; do not re-embed).
- Import/dependency information per file. The PRD's `deps.db` (Phase 2 / SQLite)
  is the eventual source for import chains + reverse imports; until it exists,
  the import-derived tiers degrade gracefully (see Notes).
- `ContextChunk` — the per-chunk struct PRD §5.3 defines (NOT yet in code; this
  task or 5.5 introduces it). Minimum fields: `chunk_id`, `rel_path`, line range,
  `content`, `score`, and an `is_expanded: bool` provenance flag.

## Outputs (must exist after completion)

- `RelatedExpander::expand(&self, results: &[HybridResult], query_vec: &[f32], store: &VectorStore) -> Result<Vec<ContextChunk>>`
  returning ONLY the newly-added expanded chunks (5.5 merges them with the direct
  hits). Each carries `is_expanded = true` and a tier-scaled `score`.
- Make `expand` `async`, with signature `async fn expand(...) -> Result<Vec<ContextChunk>>`,
  since it performs ANN calls to `store.search` per related file.
- Tiered scoring (v2.2 fix), applied to each expanded chunk's own
  vector-similarity score:
  - **0.6×** — direct imports (files imported by a result) and test files.
  - **0.4×** — reverse imports (files importing a result) and sibling
    config/barrel files (`mod.rs` / `index.ts` in the result's directory).
- Expanded chunks are **exempt from the `min_relevance` filter** (5.5 applies that
  to direct hits only) — they have already been validated as related.
- Chunk-level expansion (v2.3 fix): for each related file, run a vector-similarity
  check on *that file's* chunks against `query_vec` and include only chunks whose
  similarity > **0.3**. Never include the whole file.
- Caps: **max 5 expanded files** per query, **max 3 chunks per expanded file**.
- Hub-file skip (v2.3): skip any candidate file with **>20 inbound or outbound
  imports** (re-export barrels like `index.ts` / `mod.rs`).

## Approach

- Collect candidate related files in tier order from the result set:
  1. Direct imports + test-file matches (`test_*.py`, `*.test.ts`, `*_test.go`
     resolved against the result's path/module) → tier 0.6.
  2. Reverse imports + sibling barrel files (`mod.rs`, `index.ts` in the same
     directory) → tier 0.4.
- Drop any candidate that is already a direct hit, then drop hub files
  (>20 in/out imports). Dedup by `rel_path`, keeping the highest tier seen.
- Truncate the candidate file list to the 5-file cap (highest tier first).
- For each surviving file, `await store.search(query_vec, 3, Some("rel_path = '…'"))`
  (escape the path the same way `chunks_by_ids` does) to get its top-3 chunks
  *for this query*. Convert each `SearchResult` distance to a similarity the same
  way 4.5 does (`1.0 / (1.0 + distance)`), keep only those > 0.3, then multiply by
  the file's tier multiplier to get the final expanded `score`.
- Return the flattened `Vec<ContextChunk>`. Skipping files / chunks that fall
  below the 0.3 gate is expected — expansion is best-effort.

## Acceptance criteria

- [ ] A query hitting `src/auth/jwt.py` includes chunks from a matching
      `tests/test_auth.py` (phase exit criterion) at the 0.6× tier.
- [ ] Reverse-import and sibling-barrel candidates are scored at 0.4×; direct
      imports and test files at 0.6×.
- [ ] Expanded chunks carry `is_expanded = true` and are NOT dropped by a
      `min_relevance` filter that would remove an equally-scored direct hit.
- [ ] Chunk-level: a related file with one relevant chunk and many irrelevant
      ones contributes only the chunk(s) scoring > 0.3, never the whole file.
- [ ] Caps hold: never more than 5 expanded files, never more than 3 chunks per
      file, even when a result imports dozens of modules.
- [ ] A hub file with >20 inbound or outbound imports is skipped entirely.
- [ ] Files already present as direct hits are not re-added as expansions.
- [ ] Empty input (`results == []`) returns `[]` with no `store` calls.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test context::expander::tests::test_file_expands_at_high_tier
cargo test context::expander::tests::reverse_and_sibling_use_low_tier
cargo test context::expander::tests::chunk_level_drops_irrelevant_chunks
cargo test context::expander::tests::caps_limit_files_and_chunks
cargo test context::expander::tests::hub_files_skipped
cargo test context::expander::tests::empty_input_no_store_calls
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **Import graph dependency**: the 0.6× direct-import and 0.4× reverse-import
  tiers need `deps.db` (Phase 2). If it is not yet wired in, ship 5.3 with the
  test-file and sibling-barrel tiers (both derivable from the path alone) and
  leave the import-graph tiers behind a `deps`-availability check — they slot in
  without changing the signature. Flag clearly which tiers are live at merge time.
- **Per-file ANN scoping**: `VectorStore::search` already accepts a `filter` SQL
  predicate; scoping to `rel_path = '…'` reuses the same ANN index instead of a
  full table scan. Confirm the predicate string is escaped the way
  `chunks_by_ids` escapes `id` literals (`escape_sql_string_literal`).
- The 0.3 chunk gate and the 4.5 `semantic_relevance` mapping must agree on
  orientation (higher = better). Reuse 4.5's conversion rather than re-deriving it.
- No NEW Cargo.toml deps expected — this reuses `VectorStore` + the search crate.
