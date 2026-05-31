# Task 3.7c — VectorStore insert and `vektor index` integration

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.7c
**PRD reference**: Section 4.3 data flow, Section 4.5 delete-then-insert, Section 12 Function 3.7
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: L
**Depends on**: 3.7b, 3.10
**Blocks**: 3.8, 3.12

## Objective

Complete vector re-indexing by building LanceDB insert records from chunk metadata and vectors, then wire Phase 3 storage into both entry points — the `vektor index` CLI **and** the `index_codebase` MCP tool — while preserving Phase 2 hash/status behavior and secret-aware skips. Extract a single shared async index core so the CLI and the MCP handler are two thin callers of one path, not two divergent implementations.

## Inputs (must exist before starting)

- Reindex reuse planning from task 3.7b
- Secret detection from task 3.10
- Phase 2 `run_index` flow and `HashStore` status transitions. NOTE: `run_index` (`src/cli.rs`) is currently a **synchronous** `fn` called without `.await` from the async `run()`. It must become `async fn` (it now awaits the embedder and LanceDB), and `run()` must `.await` it.
- LanceDB schema from task 3.6
- The `handle_index_codebase` no-op stub in `src/mcp/handlers.rs` (currently a synchronous fn returning `{"status": "not implemented yet"}`)

## Outputs (must exist after completion)

- `VectorStore::reindex_file(rel_path, chunks, embedder, last_modified) -> Result<ReindexStats>`
- A shared async index core (e.g. `index_path(path, config) -> Result<IndexStats>`) that the CLI and the MCP handler both call
- LanceDB insertion in batches of at most 500 records
- `run_index` converted to `async fn`; the new module(s) declared in `src/main.rs` (`mod vector_store;`, plus `mod embedder;`/`mod secrets;` if not already added by 3.1/3.10)
- A **real** `handle_index_codebase` (async, reads a `path` from its JSON args, builds the embedder + store, calls the shared core, returns real stats JSON) — replacing the not-implemented stub. Phase 3 scope is **vector-only**; Phase 4 task 4.6 extends this same handler to also populate the Tantivy BM25 index.
- `vektor index <repo>` embeds changed chunks and writes rows under the project `lance/` directory
- CLI summary reports file, chunk, embedding, reused, skipped-secret, and failed counts
- Integration tests proving Phase 3 index writes vectors and unchanged second run skips work, plus a test exercising `handle_index_codebase` end-to-end with a fake embedder

## Approach

- Build insert records containing all PRD metadata columns: `id`, `content_hash`, `vector`, `rel_path`, `start_line`, `end_line`, `symbol_name`, `symbol_type`, `language`, `content`, and `last_modified`.
- Convert `usize` line numbers to checked `u32` values before insertion.
- Keep `--dump-chunks` read-only and free of embedding/vector writes.
- In plain `vektor index`, keep HashStore status order: set `pending` before reading/chunking/embedding, then `indexed` or `failed`.
- Apply `SecretDetector` before embedding so skipped files/chunks are never sent to ONNX or cloud APIs.
- Keep stdout concise and user-facing; detailed per-file causes belong in tracing logs.
- Factor the per-file orchestration (collect → secret-skip → reindex_file) into one async core function. The CLI's `run_index` prints a human summary; the MCP handler returns the same stats as JSON. Neither should re-implement the loop.
- The MCP handler must NOT print to stdout (stdout is the stdio-transport channel) — it returns structured JSON and logs via `tracing`.
- Convert `run_index` to `async fn` and `.await` it from `run()`. Declare any new top-level modules in `src/main.rs` alongside the existing `mod chunker; mod cli; ...` block.

## Acceptance criteria

- [ ] `VectorStore::reindex_file` inserts all current chunks for a file after deletion
- [ ] Inserts are batched at 500 records or fewer
- [ ] Inserted rows preserve chunk metadata and vector values
- [ ] `vektor index <repo>` creates a LanceDB store under the project state directory
- [ ] Running `vektor index` twice on unchanged input reports zero new embeddings
- [ ] Modifying one function reuses unchanged chunk embeddings and embeds only changed chunks
- [ ] Files or chunks flagged by `SecretDetector` are skipped before embedding and reported
- [ ] `--dump-chunks` still does not create state or vector storage
- [ ] CLI `run_index` and the MCP `index_codebase` handler call the same shared async index core (no duplicated indexing loop)
- [ ] `handle_index_codebase` is async, reads a `path` argument, indexes via the shared core, and returns real stats JSON (not the `"not implemented yet"` stub)
- [ ] The MCP handler writes nothing to stdout; it returns JSON and logs through `tracing`
- [ ] `run_index` is `async fn` and is `.await`ed from `run()`; the crate still builds with new modules declared in `src/main.rs`

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
- **Hand-off to Phase 4 (task 4.6)**: this task makes `index_codebase` real but vector-only. Task 4.6 (`index_codebase orchestrator`, depends on 3.7c + 4.2) extends the *same* shared core to also write the Tantivy BM25 index — it does not rebuild it. Keep the core factored so 4.6 is an extension, not a rewrite. (Decision: wire the handler now rather than defer wholesale to Phase 4 — recorded during the Phase 3 plan review.)
