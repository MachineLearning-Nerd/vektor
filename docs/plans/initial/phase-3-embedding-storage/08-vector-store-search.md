# Task 3.8 — VectorStore semantic search

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.8
**PRD reference**: Section 12 Function 3.8 (`VectorStore::search`)
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 3.7c
**Blocks**: 3.12, 4.5

## Objective

Implement semantic vector search over the LanceDB chunks table so Phase 4 hybrid search can consume dense retrieval results without knowing LanceDB internals.

## Inputs (must exist before starting)

- Populated chunks table from task 3.7c
- PRD search result metadata requirements
- LanceDB Rust nearest-neighbor query support for the pinned `lancedb` version

## Outputs (must exist after completion)

- `SearchResult` type containing score, chunk id, content hash, path, line range, symbol metadata, language, content, and last modified timestamp
- `VectorStore::search(query_vec: &[f32], top_k: usize, filter: Option<&str>) -> Result<Vec<SearchResult>>`
- Tests for top-k ordering, metadata extraction, dimension mismatch, and filter behavior

## Approach

- Validate query vector dimension before calling LanceDB and fail early on mismatch.
- Use LanceDB nearest-neighbor search against the `vector` column.
- Apply optional filters at query time, not after retrieving unfiltered rows.
- Convert LanceDB/Arrow rows into a storage-agnostic `SearchResult`.
- Keep RRF fusion, BM25 search, snippets, and MCP response shaping out of this task.

## Acceptance criteria

- [ ] Search returns at most `top_k` results sorted by LanceDB score
- [ ] Result metadata includes every field Phase 4 needs for hybrid fusion and display
- [ ] `top_k = 0` returns an empty list without querying
- [ ] Query vectors with the wrong dimension return a clear error
- [ ] `filter` narrows results by language or path at the LanceDB query layer
- [ ] Tests seed multiple rows and prove ranking/filtering behavior

## Verification

```bash
cargo test vector_store_search
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Phase 4 owns query embedding and BM25/RRF fusion. This task only searches an already-embedded query vector.
