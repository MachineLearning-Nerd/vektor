# Task 3.7b — VectorStore reindex reuse planning

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.7b
**PRD reference**: Section 4.5 delete-then-insert re-indexing, Section 12 Function 3.7
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 3.5, 3.7a, 3.9
**Blocks**: 3.7c

## Objective

Build the file re-indexing orchestration that reads reusable embeddings, deletes old rows, separates reused chunks from chunks that need fresh embedding, and preserves HashStore crash-recovery semantics.

## Inputs (must exist before starting)

- Embedder factory from task 3.5
- Read-before-delete cache helper from task 3.7a
- `VectorStore::delete_by_file` from task 3.9
- Phase 2 `Chunk` metadata and content-addressed IDs

## Outputs (must exist after completion)

- Internal reindex planning path that produces reused-vector records and new-embedding batches
- Fresh embedding calls only for chunks whose `content_hash` is absent from the reuse map
- Tests with unchanged and changed chunks proving cache hits avoid embedder calls

## Approach

- Call `existing_embeddings_by_content_hash(rel_path)` before `delete_by_file(rel_path)`.
- Use a fake embedder in tests that counts requested texts and returns deterministic vectors.
- Preserve chunk order for insert preparation so search result metadata remains deterministic.
- Treat empty chunk lists as a valid file state: delete old rows and insert nothing.
- Keep actual Arrow `RecordBatch` construction and insertion in task 3.7c.

## Acceptance criteria

- [ ] `reindex_file` planning reads existing embeddings before deletion
- [ ] Unchanged chunks reuse vectors and are not passed to the embedder
- [ ] Changed/new chunks are passed to the embedder in document-prefix mode
- [ ] Removed chunks disappear because delete happens before reinsert
- [ ] Empty chunk list deletes previous rows without error
- [ ] Tests prove a one-function edit embeds only changed chunks, not every chunk in the file

## Verification

```bash
cargo test reindex_reuses_embeddings
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Keep HashStore updates in the CLI/index orchestration layer, not inside `VectorStore`; storage should not decide file processing status.
