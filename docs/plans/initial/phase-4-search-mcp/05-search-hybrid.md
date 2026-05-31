# Task 4.5 — `search_hybrid(query, config)` orchestrator

**Phase**: 4 — Search + MCP
**Task ID**: 4.5
**PRD reference**: Section 4.2 (Hybrid Search + RRF Fusion)
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 3.8, 4.3, 4.4
**Blocks**: 4.6, 4.8

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
  `symbol_name`, `language`, `content`, and the combined `relevance_score` (RRF).
- `mode == semantic` → vector only; `mode == keyword` → BM25 only; `mode == hybrid`
  → both, fused.
- Concurrency: vector + keyword retrieval run via `tokio::join!` (independent I/O).

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
  single-mode, return that retriever's ranking directly (no fusion).
- Map fused `chunk_id`s back to full result rows. The semantic side already
  carries content/metadata from `VectorStore::search`; the keyword side carries
  stored fields — merge by `chunk_id`, preferring the row that has full `content`.

## Acceptance criteria

- [ ] `hybrid` mode produces a ranking that differs from `semantic`-only and
      `keyword`-only on a known query (the phase exit-criterion test).
- [ ] Adaptive weights take effect: an identifier-heavy query
      (`validate_token AuthMiddleware`) favors the BM25 contribution; a
      natural-language query (`how does authentication work`) favors the semantic
      contribution (verifiable by ranking shifts).
- [ ] Synonym expansion: querying `"auth"` surfaces chunks containing
      `"authentication"` via the BM25 leg.
- [ ] Vector + keyword retrieval run concurrently (`tokio::join!`), not serially.
- [ ] `top_k` bounds the final list; empty query → `[]`, no panic.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test search::hybrid::tests::hybrid_differs_from_single_mode
cargo test search::hybrid::tests::adaptive_weights_shift_ranking
cargo test search::hybrid::tests::synonym_expansion_recall
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **`VectorStore::search` projection (carry-over from the 3.11 review):**
  `VectorStore::search` currently does NOT `.select()` away the `vector` column,
  so every result row carries the ~3KB unused embedding. Minor at Phase 3 scale,
  but `search_hybrid` is the Phase 4 hot path, so add a projection to
  `VectorStore::search` mirroring `existing_embeddings_by_content_hash`:
  ```rust
  .select(Select::Columns(vec![
      "_distance".into(), "id".into(), "content_hash".into(), "rel_path".into(),
      "start_line".into(), "end_line".into(), "symbol_name".into(),
      "symbol_type".into(), "language".into(), "content".into(),
      "last_modified".into(),
  ]))
  ```
  i.e. select everything EXCEPT `vector` (and let `_distance` auto-project as
  today). Confirm `_distance` survives an explicit `.select()` in `lancedb 0.29`;
  if naming it in the column list double-projects or errors, omit it and rely on
  the auto-projection. Make this a small focused edit in 4.5 (or split a tiny 4.5a
  if it grows).
- For a fair semantic-vs-keyword RRF, over-fetch each retriever (e.g. `top_k * 4`,
  capped) before fusing, then truncate to `top_k`. Don't fuse only `top_k` from
  each — that loses recall.
- Test seam: inject a fake `Embedder` (as the Phase 3 MCP handler test does) so
  hybrid tests don't need a downloaded model.
