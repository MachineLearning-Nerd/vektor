# Task 3.9 — VectorStore delete_by_file

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.9
**PRD reference**: Section 4.5 delete-then-insert, Section 12 Function 3.9 (`VectorStore::delete_by_file`)
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: S
**Depends on**: 3.6
**Blocks**: 3.7b

## Objective

Add the file-scoped delete primitive needed by re-indexing, file deletion handling, and crash recovery.

## Inputs (must exist before starting)

- `VectorStore` chunks table from task 3.6
- PRD Section 4.5 delete-then-insert strategy

## Outputs (must exist after completion)

- `VectorStore::delete_by_file(rel_path: &str) -> Result<usize>` or equivalent count-returning API
- Predicate escaping or parameterization that handles paths containing quotes or backslashes
- Tests proving only matching file rows are deleted

## Approach

- Delete by exact `rel_path`, not prefix, glob, or substring.
- Return the number of deleted rows if the LanceDB API exposes it; otherwise return success and verify through follow-up query in tests.
- Treat deleting a missing file as success.
- Increment `chunks_deleted_since` stats when rows are deleted so ANN rebuild logic can use the churn counters.

## Acceptance criteria

- [ ] Rows for the target `rel_path` are deleted
- [ ] Rows for other files remain untouched
- [ ] Missing target paths do not error
- [ ] Paths containing `'`, `"`, spaces, or backslashes are handled safely
- [ ] Delete churn stats are updated when rows are removed
- [ ] Tests cover matching, non-matching, missing, and quoted-path cases

## Verification

```bash
cargo test delete_by_file
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Tantivy deletion is Phase 4. This task only deletes LanceDB vector rows.
