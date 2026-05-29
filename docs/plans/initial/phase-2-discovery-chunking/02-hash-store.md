# Task 2.2 — HashStore + SQLite state.db

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.2
**PRD reference**: Section 12 Function 1.3 (`hash_file`) and Function 1.4 (`HashStore`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.1
**Blocks**: 2.7, 2.8

## Objective

Add SHA-256 file hashing and a SQLite-backed `HashStore` for incremental indexing state. This is the only persisted Phase 2 state; no embeddings or vector store writes happen yet.

## Inputs (must exist before starting)

- File discovery from task 2.1
- `rusqlite` dependency and `Config` data directory setting
- PRD Section 12 Function 1.3 and Function 1.4 hash-store contract

## Outputs (must exist after completion)

- `hash_file` helper for file-diffing plus a full SHA-256 content-hash helper for chunk metadata and IDs
- Project-scoped SQLite `HashStore` under `config.index.data_dir`
- Unit tests for hash comparison, status transitions, and pending-row recovery

## Approach

- Resolve the configured Vektor data directory and derive a stable project-scoped state directory from the canonical repository root.
- Expand `~` in `config.index.data_dir` against the active home directory before creating state paths.
- Store one row per workspace-relative path with hash, status, and indexed timestamp.
- Keep hash calculation independent from the database so chunking can reuse full content hashes without opening SQLite.
- Query pending rows in deterministic path order.

## Acceptance criteria

- [ ] `hash_file(path) -> Result<String>` reads bytes and returns the first 16 hex chars of SHA-256
- [ ] Full SHA-256 content hashing is available for chunk `content_hash` and content-addressed chunk IDs
- [ ] `HashStore` creates `file_hashes(rel_path TEXT PRIMARY KEY, hash TEXT, status TEXT DEFAULT 'pending', indexed_at INTEGER)`
- [ ] `get_hash`, `set_hash`, `is_changed`, and `get_pending` are covered by tests
- [ ] State DB lives at a project-scoped path under the configured Vektor data directory
- [ ] `pending`, `indexed`, and `failed` status transitions are covered by tests
- [ ] Crash-recovery pending rows are preserved and returned deterministically

## Verification

```bash
cargo test hash_store
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Use `rusqlite` already pinned in `Cargo.toml`.
- Do not create LanceDB tables in this task; vector storage is Phase 3.
