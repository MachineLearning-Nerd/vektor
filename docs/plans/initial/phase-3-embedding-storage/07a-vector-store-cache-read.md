# Task 3.7a — VectorStore read-before-delete cache

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.7a
**PRD reference**: Section 4.5 read-before-delete re-indexing, Section 12 Function 3.7
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 3.6
**Blocks**: 3.7b

## Objective

Implement the read-before-delete half of file re-indexing by loading existing vectors for a file into a content-hash keyed cache before any rows are deleted.

## Inputs (must exist before starting)

- `VectorStore` and chunks table from task 3.6
- Phase 2 chunk `content_hash` semantics
- PRD Section 4.5 requirement that cache reads happen before deletes

## Outputs (must exist after completion)

- `VectorStore::existing_embeddings_by_content_hash(rel_path: &str) -> Result<HashMap<String, Vec<f32>>>`
- Tests that seed rows and verify existing embeddings are returned by `content_hash`
- Deterministic behavior when a file has no existing chunks

## Approach

- Query LanceDB by exact `rel_path`; do not scan all project chunks.
- Return a map keyed by `content_hash` because content reuse is the cache contract.
- If duplicate content hashes exist for a file, preserve one vector and log at debug level; identical content should have identical embedding.
- Keep this helper side-effect free so task 3.7b can call it before delete.
- Add tests with two files that share content hashes to prove the rel_path filter is enforced.

## Acceptance criteria

- [ ] Existing chunks for the requested `rel_path` are returned as `content_hash -> vector`
- [ ] Chunks from other files are not returned even if they share a content hash
- [ ] Missing files return an empty map
- [ ] Returned vectors preserve their full dimension and values
- [ ] Helper does not delete or mutate rows
- [ ] Tests prove read-before-delete data is still available immediately before `delete_by_file`

## Verification

```bash
cargo test existing_embeddings_by_content_hash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not implement `reindex_file` orchestration here. This task is intentionally the cache read primitive only.
