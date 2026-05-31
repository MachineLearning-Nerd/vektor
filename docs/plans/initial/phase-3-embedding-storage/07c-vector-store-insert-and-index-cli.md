# Task 3.7c — VectorStore insert and `vektor index` integration

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.7c
**PRD reference**: Section 4.3 data flow, Section 4.5 delete-then-insert, Section 12 Function 3.7
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: L
**Depends on**: 3.7b, 3.10
**Blocks**: 3.8, 3.12

## Objective

Complete vector re-indexing by building LanceDB insert records from chunk metadata and vectors, then wire Phase 3 storage into `vektor index` while preserving Phase 2 hash/status behavior and secret-aware skips.

## Inputs (must exist before starting)

- Reindex reuse planning from task 3.7b
- Secret detection from task 3.10
- Phase 2 `run_index` flow and `HashStore` status transitions
- LanceDB schema from task 3.6

## Outputs (must exist after completion)

- `VectorStore::reindex_file(rel_path, chunks, embedder, last_modified) -> Result<ReindexStats>`
- LanceDB insertion in batches of at most 500 records
- `vektor index <repo>` embeds changed chunks and writes rows under the project `lance/` directory
- CLI summary reports file, chunk, embedding, reused, skipped-secret, and failed counts
- Integration tests proving Phase 3 index writes vectors and unchanged second run skips work

## Approach

- Build insert records containing all PRD metadata columns: `id`, `content_hash`, `vector`, `rel_path`, `start_line`, `end_line`, `symbol_name`, `symbol_type`, `language`, `content`, and `last_modified`.
- Convert `usize` line numbers to checked `u32` values before insertion.
- Keep `--dump-chunks` read-only and free of embedding/vector writes.
- In plain `vektor index`, keep HashStore status order: set `pending` before reading/chunking/embedding, then `indexed` or `failed`.
- Apply `SecretDetector` before embedding so skipped files/chunks are never sent to ONNX or cloud APIs.
- Keep stdout concise and user-facing; detailed per-file causes belong in tracing logs.

## Acceptance criteria

- [ ] `VectorStore::reindex_file` inserts all current chunks for a file after deletion
- [ ] Inserts are batched at 500 records or fewer
- [ ] Inserted rows preserve chunk metadata and vector values
- [ ] `vektor index <repo>` creates a LanceDB store under the project state directory
- [ ] Running `vektor index` twice on unchanged input reports zero new embeddings
- [ ] Modifying one function reuses unchanged chunk embeddings and embeds only changed chunks
- [ ] Files or chunks flagged by `SecretDetector` are skipped before embedding and reported
- [ ] `--dump-chunks` still does not create state or vector storage

## Verification

```bash
cargo test vector_store
cargo test index_cli
cargo build
FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Use test fixtures or a fake embedder for default tests so CI does not require downloading Jina during this task.
- Real-model smoke coverage can be ignored by default if it is documented and safe to run manually.
