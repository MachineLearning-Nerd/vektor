# Task 4.5 — `search_hybrid(query, config)` orchestrator

**Phase**: 4 — Search + MCP
**Task ID**: 4.5
**PRD reference**: Section 4.2 (Hybrid Search + RRF Fusion)
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 3.8, 4.3, 4.4
**Blocks**: 4.7, 4.8

## Objective

Tie the two retrievers together: embed the query, run vector search
(`VectorStore::search`, Phase 3 task 3.8) and BM25 search (`TextIndex::search`,
4.3) concurrently, then fuse their rankings with `rrf_fuse` + `AdaptiveWeights`
(4.4). This is the function `search_code` (4.8) calls; it returns a single ranked
result list with combined RRF scores.

## Inputs (must exist before starting)

- `VectorStore::search(query_vec, top_k, filter)` from task 3.8.
- `TextIndex::search(query, top_k)` from task 4.3.
- `rrf_fuse` + `AdaptiveWeights::compute` + `SynonymExpander` from task 4.4.
- An `Embedder` (Phase 3) to embed the query (use `prefix_for_query`).
- A search mode selector (`hybrid` | `semantic` | `keyword`) — PRD §4.2 / the
  `search_code` modes. `hybrid` is the default.

## Outputs (must exist after completion)

- `search_hybrid(query, mode, top_k, store, text_index, embedder, config) -> Result<Vec<HybridResult>>`
  returning ranked results carrying `chunk_id`, `rel_path`, line range,
  `symbol_name`, `language`, `content`, and `relevance_score`. `relevance_score`
  is always higher-is-better: hybrid uses weighted RRF, keyword mode uses BM25,
  and semantic mode converts LanceDB's lower-is-better distance into a display
  relevance score (for example `1.0 / (1.0 + distance)`). Keep raw retriever
  scores only as optional debug fields.
- `mode == semantic` → vector only; `mode == keyword` → BM25 only; `mode == hybrid`
  → both, fused.
- Concurrency: vector + keyword retrieval run via `tokio::join!` (independent I/O).
- `VectorStore::search` no longer materializes the stored `vector` column on the
  Phase 4 hot path; it selects only the IDs, metadata, and `content` needed by
  `HybridResult` (the `_distance` score column auto-projects on vector queries —
  do not name it in `.select()`).
- A model-free latency smoke test fixture (fake embedder/vector store + Tantivy
  or deterministic fakes) that proves the Phase 4 `search_hybrid` hot path can
  stay under the phase exit gate: P95 <300ms on a 10K-chunk index.

## Approach

- For semantic + hybrid: prepend `embedder.prefix_for_query()`, embed the query,
  call `VectorStore::search` for a candidate pool (e.g. `top_k` over-fetched a bit
  for fusion quality).
- For keyword + hybrid: run the query through `SynonymExpander::expand` (BM25
  only), call `TextIndex::search`.
- Run the two retrievals concurrently with `tokio::join!`. (`TextIndex::search` is
  sync/CPU; wrap in `spawn_blocking` if it blocks the runtime meaningfully, or
  call it before/after the await — decide based on its cost.)
- Compute `AdaptiveWeights::compute(query)` and pass to `rrf_fuse` (hybrid). For
  single-mode, return that retriever's ranking directly (no fusion), but normalize
  score orientation so every `HybridResult.relevance_score` is higher-is-better.
  In particular, do not surface `VectorStore::SearchResult.score` directly as
  relevance; it is raw L2 distance where lower is better.
- Map fused `chunk_id`s back to full result rows. The semantic side already
  carries content/metadata from `VectorStore::search`; the keyword side carries
  stored fields — merge by `chunk_id`, preferring the row that has full `content`.
- Add the `VectorStore::search` projection in the same focused change (or split a
  tiny 4.5a if it grows): keep `_distance` available, but exclude the stored
  embedding vector from result batches.

## Acceptance criteria

- [ ] `hybrid` mode produces a ranking that differs from `semantic`-only and
      `keyword`-only on a known query (the phase exit-criterion test).
- [ ] Adaptive weights take effect: an identifier-heavy query
      (`validate_token AuthMiddleware`) favors the BM25 contribution; a
      natural-language query (`how does authentication work`) favors the semantic
      contribution (verifiable by ranking shifts).
- [ ] `HybridResult.relevance_score` is higher-is-better in all modes; semantic
      mode inverts/normalizes LanceDB distance instead of returning raw distance
      as display relevance.
- [ ] Synonym expansion: querying `"auth"` surfaces chunks containing
      `"authentication"` via the BM25 leg.
- [ ] Vector + keyword retrieval run concurrently (`tokio::join!`), not serially.
- [ ] `VectorStore::search` does not fetch/materialize the stored `vector`
      column for result rows, and `_distance` still survives the projection.
- [ ] Search latency P95 is <300ms on a 10K-chunk fixture, matching the phase
      exit criterion. Keep this as a deterministic ignored/perf test if it is too
      slow for every local test run, but it must be runnable before phase closure.
- [ ] `top_k` bounds the final list; empty query → `[]`, no panic.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test search::hybrid::tests::hybrid_differs_from_single_mode
cargo test search::hybrid::tests::adaptive_weights_shift_ranking
cargo test search::hybrid::tests::semantic_mode_reports_high_is_better_relevance
cargo test search::hybrid::tests::synonym_expansion_recall
cargo test vector_store::tests::search_projection_excludes_vector_column
cargo test search::hybrid::tests::ten_k_chunk_latency_p95_under_300ms -- --ignored --nocapture
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **`VectorStore::search` projection (carry-over from the 3.11 review):**
  `VectorStore::search` currently does NOT `.select()` away the `vector` column,
  so every result row carries the ~3KB unused embedding. Minor at Phase 3 scale,
  but `search_hybrid` is the Phase 4 hot path. Add a projection to
  `VectorStore::search` using the same `.select(Select::Columns(...))` technique as
  `existing_embeddings_by_content_hash`:
  ```rust
  .select(Select::Columns(vec![
      "id".into(), "content_hash".into(), "rel_path".into(),
      "start_line".into(), "end_line".into(), "symbol_name".into(),
      "symbol_type".into(), "language".into(), "content".into(),
      "last_modified".into(),
  ]))
  ```
  i.e. select everything EXCEPT `vector`. Do NOT name `_distance` in the column
  list — it auto-projects on every vector query (`disable_scoring_autoprojection`
  defaults to false; see the comment in `src/vector_store/mod.rs` `search`), and
  naming it explicitly risks a double-projection error. Make this a small focused
  edit in 4.5 (or split a tiny 4.5a if it grows).
- For a fair semantic-vs-keyword RRF, over-fetch each retriever (e.g. `top_k * 4`,
  capped) before fusing, then truncate to `top_k`. Don't fuse only `top_k` from
  each — that loses recall.
- Test seam: inject a fake `Embedder` (as the Phase 3 MCP handler test does) so
  hybrid tests don't need a downloaded model.
